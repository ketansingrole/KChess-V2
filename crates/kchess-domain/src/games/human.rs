//! Playing like people of a rating. In the opening the computer plays the moves people of that
//! rating played from the position, as often as they played them (`from_games`, from the Lichess
//! opening explorer). After it, one search compares a few lines (`LINES`) and `pick` plays one,
//! the way a person of that rating would choose: each line's loss is the expected score it gives
//! up against the best line, by the curve the reviews judge moves by, and a line is played with a
//! weight of `exp(-loss / temperature)`, the temperature falling as the rating rises. So a 1200
//! often plays a move a pawn worse and now and then a real blunder, while a 2200 rarely leaves
//! the best two. A mate found is always played, and from 1800 on a move that lets the opponent
//! mate at once never is.

use crate::review::winning_chances;

/// The rating bands offered, as the Lichess opening explorer groups its players: each is the
/// lower bound of its band.
pub const BANDS: [u32; 8] = [1000, 1200, 1400, 1600, 1800, 2000, 2200, 2500];
/// The opening, in moves made by both sides, in which people's moves are looked up: to move 20.
pub const OPENING_PLIES: usize = 40;
/// Fewer games than this from a position are too rare to copy: from there the search plays.
pub const MIN_GAMES: u64 = 10;
/// How many lines a search compares (MultiPV).
pub const LINES: u32 = 5;
/// From this rating on, a move that lets the opponent mate at once is never played.
pub const SEES_MATE_IN_ONE: u32 = 1800;
/// The temperature of people rated 1000 and 2500; others lie on the geometric curve between
/// (and beyond) them.
const TEMPERATURE_AT_1000: f64 = 0.12;
const TEMPERATURE_AT_2500: f64 = 0.008;
/// The ratings the formulas are held within.
const LOWEST: u32 = 400;
const HIGHEST: u32 = 3000;

/// What the computer is called when it plays like people of `band`.
pub fn name(band: u32) -> String {
    format!("Players rated {band}")
}

/// Whether `band` is one of the bands offered.
pub fn is_band(band: u32) -> bool {
    BANDS.contains(&band)
}

/// One of people's moves from `moves` (each with how many games played it), drawn as often as
/// they played it by `roll` in [0, 1); none when the position has fewer than `MIN_GAMES` games.
pub fn from_games(moves: &[(String, u64)], roll: f64) -> Option<String> {
    let total: u64 = moves.iter().map(|(_, games)| games).sum();
    if total < MIN_GAMES {
        return None;
    }
    super::computer::pick_from_games(moves, roll)
}

/// How long the search thinks for people of `rating`, in milliseconds: a quick look for a
/// beginner, longer for a strong player.
pub fn movetime(rating: u32) -> u64 {
    200 + u64::from(rating.clamp(LOWEST, HIGHEST) - LOWEST) / 4
}

/// The depth the search stops at for people of `rating`: a shallow search misjudges the deeper
/// tactics, as they do.
pub fn depth(rating: u32) -> u32 {
    6 + (rating.clamp(LOWEST, HIGHEST) - LOWEST) / 200
}

/// One line the search found: its first move and its score from the side to move's view.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub uci: String,
    /// Centipawns, when there is no forced mate.
    pub cp: Option<f64>,
    /// Moves to mate: positive when the side to move mates, negative when it is mated.
    pub mate: Option<f64>,
}

impl Candidate {
    /// The expected score of the line for the side to move, from 0 (lost) to 1 (won).
    pub fn expected_score(&self) -> f64 {
        (winning_chances(self.cp, self.mate) + 1.0) / 2.0
    }

    fn mates(&self) -> Option<f64> {
        self.mate.filter(|mate| *mate > 0.0)
    }

    /// The opponent mates on their next move.
    fn hangs_mate_in_one(&self) -> bool {
        self.mate.is_some_and(|mate| (-1.0..0.0).contains(&mate))
    }
}

/// How freely people of `rating` stray from the best line: the higher, the more often they play
/// a worse one.
pub fn temperature(rating: u32) -> f64 {
    let rating = f64::from(rating.clamp(LOWEST, HIGHEST));
    let along = (rating - 1000.0) / 1500.0;
    TEMPERATURE_AT_1000 * (TEMPERATURE_AT_2500 / TEMPERATURE_AT_1000).powf(along)
}

/// Each candidate's chance of being played by people of `rating`, in order; they add up to 1
/// unless there are none. A move that appears twice is played by its first line only.
pub fn chances(candidates: &[Candidate], rating: u32) -> Vec<f64> {
    let playable: Vec<bool> = candidates
        .iter()
        .enumerate()
        .map(|(at, candidate)| {
            !candidate.uci.is_empty()
                && candidates
                    .iter()
                    .take(at)
                    .all(|earlier| earlier.uci != candidate.uci)
        })
        .collect();
    // A mate found is always played, the quickest first.
    let mate = candidates
        .iter()
        .zip(&playable)
        .enumerate()
        .filter(|(_, (_, playable))| **playable)
        .filter_map(|(at, (candidate, _))| Some((at, candidate.mates()?)))
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(at, _)| at);
    if let Some(mate) = mate {
        return (0..candidates.len())
            .map(|at| if at == mate { 1.0 } else { 0.0 })
            .collect();
    }
    // Strong players see a mate in one coming, unless every move allows it.
    let sees_mate = rating >= SEES_MATE_IN_ONE
        && candidates
            .iter()
            .zip(&playable)
            .any(|(candidate, playable)| *playable && !candidate.hangs_mate_in_one());
    let allowed: Vec<bool> = candidates
        .iter()
        .zip(&playable)
        .map(|(candidate, playable)| *playable && !(sees_mate && candidate.hangs_mate_in_one()))
        .collect();
    let scores: Vec<f64> = candidates.iter().map(Candidate::expected_score).collect();
    let best = scores
        .iter()
        .zip(&allowed)
        .filter(|(_, allowed)| **allowed)
        .map(|(score, _)| *score)
        .fold(f64::NEG_INFINITY, f64::max);
    let temperature = temperature(rating);
    let weights: Vec<f64> = scores
        .iter()
        .zip(&allowed)
        .map(|(score, allowed)| {
            if *allowed {
                (-(best - score) / temperature).exp()
            } else {
                0.0
            }
        })
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 || !total.is_finite() {
        return vec![0.0; candidates.len()];
    }
    weights.into_iter().map(|weight| weight / total).collect()
}

/// The move people of `rating` play among `candidates`, drawn by `roll` in [0, 1); none when
/// there are no candidates.
pub fn pick(candidates: &[Candidate], rating: u32, roll: f64) -> Option<String> {
    let chances = chances(candidates, rating);
    let roll = roll.clamp(0.0, 1.0);
    let mut reached = 0.0;
    let mut last = None;
    for (candidate, chance) in candidates.iter().zip(chances) {
        if chance <= 0.0 {
            continue;
        }
        reached += chance;
        last = Some(candidate);
        if roll < reached {
            return Some(candidate.uci.clone());
        }
    }
    // A roll at 1, or a sum that rounds below it.
    last.map(|candidate| candidate.uci.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_testkit::random::{self, Rng};

    fn line(uci: &str, cp: f64) -> Candidate {
        Candidate {
            uci: uci.into(),
            cp: Some(cp),
            mate: None,
        }
    }

    fn mating(uci: &str, mate: f64) -> Candidate {
        Candidate {
            uci: uci.into(),
            cp: None,
            mate: Some(mate),
        }
    }

    /// A quiet position's five lines: two good moves, a fair one, a pawn's mistake and a blunder.
    fn quiet() -> Vec<Candidate> {
        vec![
            line("e2e4", 40.0),
            line("d2d4", 25.0),
            line("g1f3", 0.0),
            line("a2a3", -60.0),
            line("f2f3", -180.0),
        ]
    }

    #[test]
    fn the_bands_are_the_explorers_rating_groups() {
        // Lichess's groups: 400, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2500.
        let lichess = [400, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2500];
        assert!(BANDS.iter().all(|band| lichess.contains(band)));
        assert!(is_band(1600) && !is_band(1500));
        assert_eq!(name(1600), "Players rated 1600");
    }

    #[test]
    fn stronger_people_stray_less_and_search_deeper() {
        for pair in BANDS.windows(2) {
            let [weaker, stronger] = pair else {
                unreachable!("windows of two")
            };
            assert!(temperature(*weaker) > temperature(*stronger), "{pair:?}");
            assert!(movetime(*weaker) < movetime(*stronger), "{pair:?}");
            assert!(depth(*weaker) < depth(*stronger), "{pair:?}");
        }
        assert!((temperature(1000) - TEMPERATURE_AT_1000).abs() < 1e-12);
        assert!((temperature(2500) - TEMPERATURE_AT_2500).abs() < 1e-12);
        // Held within the ratings the formulas are made for.
        assert_eq!(temperature(100), temperature(LOWEST));
        assert_eq!(depth(4000), depth(HIGHEST));
    }

    #[test]
    fn the_lowest_roll_plays_the_best_line_and_the_highest_the_worst_allowed() {
        let lines = quiet();
        for band in BANDS {
            assert_eq!(pick(&lines, band, 0.0).as_deref(), Some("e2e4"), "{band}");
        }
        assert_eq!(pick(&lines, 1000, 1.0).as_deref(), Some("f2f3"));
        // The chances add up and fall with the loss.
        let chances = chances(&lines, 1200);
        assert!((chances.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(
            chances.windows(2).all(|pair| pair[0] > pair[1]),
            "{chances:?}"
        );
        assert_eq!(pick(&[], 1500, 0.5), None);
    }

    #[test]
    fn a_mate_found_is_always_played_the_quickest_first() {
        let lines = vec![
            line("e2e4", 500.0),
            mating("d1h5", 3.0),
            mating("h5f7", 1.0),
            line("a2a3", 400.0),
        ];
        for band in BANDS {
            for roll in [0.0, 0.3, 0.7, 0.99, 1.0] {
                assert_eq!(pick(&lines, band, roll).as_deref(), Some("h5f7"), "{band}");
            }
        }
    }

    #[test]
    fn from_1800_a_move_that_hangs_mate_in_one_is_never_played() {
        let lines = vec![line("e2e4", 0.0), line("d2d4", -10.0), mating("g2g4", -1.0)];
        assert!(chances(&lines, 1600)[2] > 0.0);
        assert_eq!(pick(&lines, 1000, 1.0).as_deref(), Some("g2g4"));
        for band in [1800, 2000, 2200, 2500] {
            assert_eq!(chances(&lines, band)[2], 0.0, "{band}");
            assert_ne!(pick(&lines, band, 1.0).as_deref(), Some("g2g4"), "{band}");
        }
        // When every move allows it, the best of them is still played.
        let lost = vec![mating("h1g1", -1.0), mating("h1h2", -1.0)];
        assert_eq!(pick(&lost, 2200, 0.0).as_deref(), Some("h1g1"));
    }

    #[test]
    fn a_move_twice_is_played_by_its_first_line_only() {
        let lines = vec![line("e2e4", 30.0), line("e2e4", 30.0), line("d2d4", 30.0)];
        let chances = chances(&lines, 1500);
        assert_eq!(chances[1], 0.0);
        assert!((chances[0] - 0.5).abs() < 1e-9 && (chances[2] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn peoples_moves_are_copied_only_from_positions_with_enough_games() {
        let rare = vec![("e2e4".to_string(), 6), ("d2d4".to_string(), 3)];
        assert_eq!(from_games(&rare, 0.0), None);
        let common = vec![("e2e4".to_string(), 7), ("d2d4".to_string(), 3)];
        assert_eq!(from_games(&common, 0.0).as_deref(), Some("e2e4"));
        assert_eq!(from_games(&common, 0.75).as_deref(), Some("d2d4"));
    }

    /// A roll in [0, 1) from the seeded generator.
    fn roll(rng: &mut Rng) -> f64 {
        (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Over a thousand draws in the quiet position, stronger bands play the best line more often
    /// and give up less, a 2200 rarely leaves the best two, and a 1200 often plays a move
    /// 50–150 cp worse or a blunder.
    #[test]
    fn random_draws_play_closer_to_the_best_as_the_rating_rises() {
        let lines = quiet();
        let best = lines[0].expected_score();
        random::check(8, |rng| {
            let draws = 1000;
            let mut best_rate = Vec::new();
            let mut average_loss = Vec::new();
            for band in BANDS {
                let mut counts = [0usize; 5];
                let mut loss = 0.0;
                for _ in 0..draws {
                    let uci = pick(&lines, band, roll(rng)).expect("a move");
                    let at = lines
                        .iter()
                        .position(|line| line.uci == uci)
                        .expect("one of the lines");
                    counts[at] += 1;
                    loss += best - lines[at].expected_score();
                }
                let share = |count: usize| count as f64 / draws as f64;
                best_rate.push(share(counts[0]));
                average_loss.push(loss / draws as f64);
                match band {
                    1200 => assert!(
                        share(counts[3] + counts[4]) > 0.08,
                        "a 1200 makes mistakes: {counts:?}"
                    ),
                    2200 => assert!(
                        share(counts[2] + counts[3] + counts[4]) < 0.08,
                        "a 2200 keeps to the best two: {counts:?}"
                    ),
                    _ => {}
                }
            }
            // 1200 against 2200.
            assert!(best_rate[6] > best_rate[1] + 0.2, "{best_rate:?}");
            // Every other band, so draws' noise cannot reorder neighbours.
            for pair in [0usize, 2, 4, 6].windows(2) {
                let (weaker, stronger) = (pair[0], pair[1]);
                assert!(
                    average_loss[weaker] > average_loss[stronger],
                    "{average_loss:?}"
                );
                assert!(best_rate[weaker] < best_rate[stronger], "{best_rate:?}");
            }
        });
    }
}
