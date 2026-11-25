//! A tournament's table from its games: which games one edition of an event holds, each
//! player's points, the Sonneborn–Berger tiebreak and the FIDE performance rating.

use super::Outcome;

/// Games of one event name more than this many days apart belong to different editions: the
/// longest pause a single event takes, a month between a team league's rounds included, stays
/// well inside it, while the next year's edition is far beyond it.
pub const EDITION_GAP: u32 = 60;

/// A day number for a `YYYYMMDD` date key, close enough to the calendar to measure gaps of weeks:
/// every month counts 31 days, an unknown month or day its first. `None` when the year is unknown.
pub fn day(date_key: u32) -> Option<u32> {
    let year = date_key / 10_000;
    if year == 0 {
        return None;
    }
    let month = (date_key / 100 % 100).max(1) - 1;
    let day = (date_key % 100).max(1) - 1;
    Some(year * 372 + month * 31 + day)
}

/// Which of an event's games, by their day numbers, are the edition of game `chosen`: the games
/// no gap longer than `EDITION_GAP` days separates from it. The undated games are an edition of
/// their own.
pub fn edition(days: &[Option<u32>], chosen: usize) -> Vec<usize> {
    let Some(at) = days[chosen] else {
        return (0..days.len()).filter(|&i| days[i].is_none()).collect();
    };
    let mut dated: Vec<(u32, usize)> = days
        .iter()
        .enumerate()
        .filter_map(|(i, day)| day.map(|day| (day, i)))
        .collect();
    dated.sort_unstable();
    let mut from = 0;
    for next in 1..=dated.len() {
        let ends = next == dated.len() || dated[next].0 - dated[next - 1].0 > EDITION_GAP;
        if !ends {
            continue;
        }
        let run = &dated[from..next];
        if run.first().is_some_and(|first| first.0 <= at)
            && run.last().is_some_and(|last| at <= last.0)
        {
            let mut games: Vec<usize> = run.iter().map(|(_, i)| *i).collect();
            games.sort_unstable();
            return games;
        }
        from = next;
    }
    vec![chosen]
}

/// A finished game between players numbered from 0, with their ratings in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Played {
    pub white: usize,
    pub black: usize,
    pub outcome: Outcome,
    pub white_elo: Option<u16>,
    pub black_elo: Option<u16>,
}

/// A player's standing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Standing {
    pub player: usize,
    /// Points in halves.
    pub half_points: u32,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    /// Sonneborn–Berger in quarter points.
    pub tiebreak_quarters: u32,
    pub performance: Option<u32>,
}

/// The half points `outcome` gives White and Black; `None` for a game with no result.
pub fn half_points(outcome: Outcome) -> Option<(u32, u32)> {
    match outcome {
        Outcome::White => Some((2, 0)),
        Outcome::Draw => Some((1, 1)),
        Outcome::Black => Some((0, 2)),
        Outcome::Unknown => None,
    }
}

/// The standings of `players` players from their finished games: the most points first, then the
/// higher Sonneborn–Berger, then the more wins, then the lower number.
pub fn standings(players: usize, games: &[Played]) -> Vec<Standing> {
    let mut table: Vec<Standing> = (0..players)
        .map(|player| Standing {
            player,
            ..Standing::default()
        })
        .collect();
    // Each player's rated opponents' ratings and their points against them, in halves.
    let mut rated: Vec<(u64, u32, u32)> = vec![(0, 0, 0); players];
    let scored: Vec<(Played, u32, u32)> = games
        .iter()
        .filter_map(|game| half_points(game.outcome).map(|(w, b)| (*game, w, b)))
        .collect();
    for (game, white, black) in &scored {
        for (me, points, against) in [
            (game.white, *white, game.black_elo),
            (game.black, *black, game.white_elo),
        ] {
            let standing = &mut table[me];
            standing.half_points += points;
            match points {
                2 => standing.wins += 1,
                1 => standing.draws += 1,
                _ => standing.losses += 1,
            }
            if let Some(elo) = against {
                let (sum, count, earned) = &mut rated[me];
                *sum += u64::from(elo);
                *count += 1;
                *earned += points;
            }
        }
    }
    let points: Vec<u32> = table.iter().map(|standing| standing.half_points).collect();
    for (game, white, black) in &scored {
        // A win counts the loser's points, a draw half of them: in quarters, twice and once
        // their half points.
        table[game.white].tiebreak_quarters += white * points[game.black];
        table[game.black].tiebreak_quarters += black * points[game.white];
    }
    for (standing, (sum, count, earned)) in table.iter_mut().zip(rated) {
        if count > 0 {
            let average = sum as f64 / f64::from(count);
            let fraction = f64::from(earned) / f64::from(2 * count);
            standing.performance = u32::try_from(performance(average, fraction)).ok();
        }
    }
    table.sort_by(|a, b| {
        (b.half_points, b.tiebreak_quarters, b.wins, a.player).cmp(&(
            a.half_points,
            a.tiebreak_quarters,
            a.wins,
            b.player,
        ))
    });
    table
}

/// FIDE's rating differences for a score of 50% to 100%, a step of 1% each (FIDE Rating
/// Regulations, table 8.1a); below 50% they are the same, negative.
const DP: [i32; 51] = [
    0, 7, 14, 21, 29, 36, 43, 50, 57, 65, 72, 80, 87, 95, 102, 110, 117, 125, 133, 141, 149, 158,
    166, 175, 184, 193, 202, 211, 220, 230, 240, 251, 262, 273, 284, 296, 309, 322, 336, 351, 366,
    383, 401, 422, 444, 470, 501, 538, 589, 677, 800,
];

/// The FIDE performance rating of a score `fraction` (0 to 1) against opponents rated `average`.
pub fn performance(average: f64, fraction: f64) -> i32 {
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).round() as i32;
    let difference = if percent >= 50 {
        DP[(percent - 50) as usize]
    } else {
        -DP[(50 - percent) as usize]
    };
    average.round() as i32 + difference
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(white: usize, black: usize, outcome: Outcome) -> Played {
        Played {
            white,
            black,
            outcome,
            white_elo: Some(2000),
            black_elo: Some(2000),
        }
    }

    #[test]
    fn a_day_counts_months_of_31_days_and_an_unknown_month_or_day_as_its_first() {
        assert_eq!(day(0), None);
        assert_eq!(day(20240000), day(20240101));
        assert_eq!(day(20240201).unwrap() - day(20240101).unwrap(), 31);
        assert_eq!(day(20250101).unwrap() - day(20240101).unwrap(), 372);
    }

    #[test]
    fn an_event_name_splits_into_editions_at_long_gaps() {
        let days = [
            day(20231228),
            day(20240105),
            day(20241228),
            None,
            day(20231229),
            day(20250104),
            None,
        ];
        assert_eq!(edition(&days, 0), vec![0, 1, 4]);
        assert_eq!(edition(&days, 1), vec![0, 1, 4]);
        assert_eq!(edition(&days, 5), vec![2, 5]);
        assert_eq!(edition(&days, 3), vec![3, 6]);
    }

    #[test]
    fn standings_order_by_points_then_sonneborn_berger() {
        // 0 beats 1, 1 beats 2, 2 draws 0, 3 loses to everyone but draws 1.
        let games = [
            game(0, 1, Outcome::White),
            game(1, 2, Outcome::White),
            game(2, 0, Outcome::Draw),
            game(3, 0, Outcome::Black),
            game(1, 3, Outcome::Draw),
            game(3, 2, Outcome::Black),
            game(2, 3, Outcome::Unknown),
        ];
        let table = standings(4, &games);
        let order: Vec<usize> = table.iter().map(|s| s.player).collect();
        // 0 has 2½; 1 and 2 have 1½ each; 3 has ½.
        assert_eq!(order, vec![0, 1, 2, 3]);
        let first = &table[0];
        assert_eq!(
            (first.half_points, first.wins, first.draws, first.losses),
            (5, 2, 1, 0)
        );
        // 0 beat 1 (1½) and 3 (½), drew 2 (1½): 1½ + ½ + ¾ = 2¾.
        assert_eq!(first.tiebreak_quarters, 11);
        // 1 and 2 have 1½ each: 1 beat 2 (1½) and drew 3 (½): 1¾; 2 beat 3 (½) and drew 0
        // (2½): 1¾ too, and as many wins, so the lower number goes first.
        assert_eq!(table[1].tiebreak_quarters, 7);
        assert_eq!(table[2].tiebreak_quarters, 7);
        assert_eq!(table[3].half_points, 1);
    }

    #[test]
    fn performance_follows_fides_table() {
        assert_eq!(performance(2000.0, 0.5), 2000);
        assert_eq!(performance(2000.0, 1.0), 2800);
        assert_eq!(performance(2000.0, 0.0), 1200);
        assert_eq!(performance(2400.4, 0.75), 2593);
        assert_eq!(performance(2400.0, 0.25), 2207);
        let table = standings(2, &[game(0, 1, Outcome::White)]);
        assert_eq!(table[0].performance, Some(2800));
        assert_eq!(table[1].performance, Some(1200));
        let unrated = Played {
            white_elo: None,
            black_elo: None,
            ..game(0, 1, Outcome::Draw)
        };
        assert_eq!(standings(2, &[unrated])[0].performance, None);
    }
}
