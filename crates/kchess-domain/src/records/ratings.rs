//! Rating-history keys, display names, histories rebuilt from
//! games and the merge of Lichess's own histories with them.

use crate::perf::Speed;
use crate::variant::Variant;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::js;

const DAY_MS: i64 = 86_400_000;

const fn named(variant: Variant) -> (&'static str, &'static str) {
    (variant.as_str(), variant.label())
}

const fn speed(speed: Speed) -> (&'static str, &'static str) {
    (speed.as_str(), speed.label())
}

/// The label for each rating-history key.
pub const DISPLAY_NAMES: &[(&str, &str)] = &[
    speed(Speed::UltraBullet),
    speed(Speed::Bullet),
    speed(Speed::Blitz),
    speed(Speed::Rapid),
    speed(Speed::Classical),
    speed(Speed::Correspondence),
    named(Variant::Chess960),
    named(Variant::Crazyhouse),
    named(Variant::Antichess),
    named(Variant::Atomic),
    named(Variant::Horde),
    named(Variant::KingOfTheHill),
    named(Variant::RacingKings),
    named(Variant::ThreeCheck),
    ("puzzle", "Puzzles"),
];

/// Lowercase names without spaces, `_` or `-`, to their key.
const KEY_BY_NORMALIZED: &[(&str, &str)] = &[
    ("ultrabullet", "ultraBullet"),
    ("bullet", "bullet"),
    ("blitz", "blitz"),
    ("rapid", "rapid"),
    ("classical", "classical"),
    ("correspondence", "correspondence"),
    ("chess960", "chess960"),
    ("crazyhouse", "crazyhouse"),
    ("antichess", "antichess"),
    ("atomic", "atomic"),
    ("horde", "horde"),
    ("kingofthehill", "kingOfTheHill"),
    ("racingkings", "racingKings"),
    ("threecheck", "threeCheck"),
    ("puzzle", "puzzle"),
    ("puzzles", "puzzle"),
];

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// The rating-history key of a Lichess perf or history name ("King of the Hill" is
/// `kingOfTheHill`); an unknown name is kept as it is.
pub fn normalize_rating_key(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let key: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !js::is_space(*c) && *c != '_' && *c != '-')
        .collect();
    lookup(KEY_BY_NORMALIZED, &key).map_or_else(|| name.to_string(), str::to_string)
}

/// The label of a rating category's key ("kingOfTheHill" is "King of the Hill").
pub fn perf_label(key: &str) -> Option<&'static str> {
    DISPLAY_NAMES
        .iter()
        .find(|(known, _)| *known == key)
        .map(|(_, label)| *label)
}

/// The label of a rating history, or the name itself when it has none.
pub fn rating_display_name(name: &str) -> String {
    lookup(DISPLAY_NAMES, &normalize_rating_key(name))
        .map_or_else(|| name.to_string(), str::to_string)
}

/// Whether a rating history is the puzzle rating's.
pub fn is_puzzle_history(name: &str) -> bool {
    normalize_rating_key(name) == "puzzle"
}

/// One rating category's rating, the last rated one each day: `[year, month (0–11), day,
/// rating]`, as Lichess sends its rating history.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RatingSeries {
    pub name: String,
    pub points: Vec<[i64; 4]>,
}

impl RatingSeries {
    /// Its rating-history key: `blitz`, `puzzle`, `kingOfTheHill`…
    pub fn key(&self) -> String {
        normalize_rating_key(&self.name)
    }

    /// The puzzle rating's, which is no speed of play.
    pub fn is_puzzles(&self) -> bool {
        is_puzzle_history(&self.name)
    }
}

/// A rated game as a rating history counts it: when it was played, its rating category and the
/// player's rating after it.
#[derive(Clone, Copy, Debug)]
pub struct RatedGame<'a> {
    pub created_at: i64,
    pub perf: &'a str,
    pub rating: i64,
}

/// Rating histories rebuilt from rated games: one point per UTC day per rating category, the
/// day's last game, the categories in the order first played.
pub fn rating_series<'a>(games: impl IntoIterator<Item = RatedGame<'a>>) -> Vec<RatingSeries> {
    let mut games: Vec<RatedGame> = games.into_iter().collect();
    games.sort_by_key(|game| game.created_at);
    let mut series: Vec<(String, Vec<(i64, i64)>)> = Vec::new();
    for game in games {
        let key = normalize_rating_key(game.perf);
        let day = game.created_at.div_euclid(DAY_MS);
        let index = match series.iter().position(|(name, _)| *name == key) {
            Some(index) => index,
            None => {
                series.push((key, Vec::new()));
                series.len() - 1
            }
        };
        let days = &mut series[index].1;
        match days.last_mut() {
            Some(last) if last.0 == day => last.1 = game.rating,
            _ => days.push((day, game.rating)),
        }
    }
    series
        .into_iter()
        .map(|(name, days)| RatingSeries {
            name,
            points: days
                .into_iter()
                .filter_map(|(day, rating)| {
                    let (year, month, date) = crate::calendar::civil_from_days(day)?;
                    Some([year, month - 1, date, rating])
                })
                .collect(),
        })
        .collect()
}

/// Lichess's rating histories, then the ones rebuilt from games for the categories Lichess has
/// no points for.
pub fn merge_rating_series(
    official: &[RatingSeries],
    from_games: &[RatingSeries],
) -> Vec<RatingSeries> {
    let covered: HashSet<String> = official
        .iter()
        .filter(|series| !series.points.is_empty())
        .map(RatingSeries::key)
        .collect();
    official
        .iter()
        .chain(
            from_games
                .iter()
                .filter(|series| !covered.contains(&series.key())),
        )
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A series is told by its name, as Lichess writes it: only the puzzle rating's is puzzles.
    #[test]
    fn a_series_knows_its_key_and_whether_it_is_puzzles() {
        let series = |name: &str| RatingSeries {
            name: name.into(),
            points: Vec::new(),
        };
        assert!(series("Puzzles").is_puzzles());
        assert!(!series("Blitz").is_puzzles());
        assert_eq!(series("King of the Hill").key(), "kingOfTheHill");
    }

    #[test]
    fn keys_normalize_like_lichess_names() {
        assert_eq!(normalize_rating_key("King of the Hill"), "kingOfTheHill");
        assert_eq!(normalize_rating_key("Puzzles"), "puzzle");
        assert_eq!(normalize_rating_key("Unknown_Thing"), "Unknown_Thing");
        assert_eq!(normalize_rating_key(""), "");
    }

    const DAY: i64 = DAY_MS;

    fn game(created_at: i64, perf: &str, rating: i64) -> RatedGame<'_> {
        RatedGame {
            created_at,
            perf,
            rating,
        }
    }

    /// A day's last game is its point, whatever order the games come in; each category keeps
    /// its own points, in the order first played.
    #[test]
    fn a_days_last_game_is_its_point() {
        let series = rating_series([
            game(DAY + 50, "blitz", 1510),
            game(DAY + 10, "Blitz", 1500),
            game(DAY + 20, "rapid", 1700),
            game(3 * DAY, "blitz", 1490),
        ]);
        assert_eq!(
            series,
            [
                RatingSeries {
                    name: "blitz".into(),
                    points: vec![[1970, 0, 2, 1510], [1970, 0, 4, 1490]],
                },
                RatingSeries {
                    name: "rapid".into(),
                    points: vec![[1970, 0, 2, 1700]],
                },
            ]
        );
    }

    /// Games fill only the categories Lichess has no points for.
    #[test]
    fn lichess_points_win_over_games() {
        let series = |name: &str, points: Vec<[i64; 4]>| RatingSeries {
            name: name.into(),
            points,
        };
        let official = [
            series("Blitz", vec![[2026, 0, 1, 1500]]),
            series("Rapid", vec![]),
        ];
        let from_games = [
            series("blitz", vec![[2026, 0, 2, 1400]]),
            series("rapid", vec![[2026, 0, 2, 1600]]),
        ];
        let merged = merge_rating_series(&official, &from_games);
        let names: Vec<&str> = merged.iter().map(|series| series.name.as_str()).collect();
        assert_eq!(names, ["Blitz", "Rapid", "rapid"]);
    }
}
