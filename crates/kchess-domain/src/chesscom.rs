//! Chess.com games as KChess keeps synced games: a game from a player's monthly archive
//! (`api.chess.com/pub/player/{name}/games/{yyyy}/{mm}`) becomes the row a Lichess game becomes,
//! with Lichess's words for its speed, variant and how it ended, so History, the review queue,
//! insights and mistake drills read both alike.
//!
//! A Chess.com player's games are kept under the account `name@chess.com` (a Lichess name has no
//! `@`), and each game's id is `cc-l<number>` (live) or `cc-d<number>` (daily): a Lichess id has
//! no dash, so ids from the two sites never meet.

use serde_json::{Value, json};

use crate::pgn::{Game, parse_pgn};
use crate::review::san_to_uci;
use crate::rules;
use crate::training::openings::opening_at;

/// The suffix of a Chess.com player's account.
pub const ACCOUNT_SUFFIX: &str = "@chess.com";

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// `^[a-zA-Z0-9_-]{3,25}$`: what Chess.com accepts as a name.
pub fn is_username(name: &str) -> bool {
    (3..=25).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The account a Chess.com player's games are kept under.
pub fn account_of(username: &str) -> String {
    format!("{username}{ACCOUNT_SUFFIX}")
}

/// The Chess.com name of an account, when it is a Chess.com one.
pub fn username_of(account: &str) -> Option<&str> {
    let name = account.get(..account.len().checked_sub(ACCOUNT_SUFFIX.len())?)?;
    let suffix = account.get(name.len()..)?;
    (suffix.eq_ignore_ascii_case(ACCOUNT_SUFFIX) && is_username(name)).then_some(name)
}

/// `^cc-[ld][0-9]{1,20}$`: a Chess.com game's id as KChess keeps it.
pub fn is_game_id(id: &str) -> bool {
    id.strip_prefix("cc-l")
        .or_else(|| id.strip_prefix("cc-d"))
        .is_some_and(|number| {
            (1..=20).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_digit())
        })
}

/// A game's id from its link: `https://www.chess.com/game/live/123` is `cc-l123`.
pub fn game_id(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?.trim_end_matches('/');
    let mut parts = path.rsplit('/');
    let number = parts.next()?;
    let kind = match parts.next()? {
        "live" => 'l',
        "daily" => 'd',
        _ => return None,
    };
    let id = format!("cc-{kind}{number}");
    is_game_id(&id).then_some(id)
}

/// Lichess's speed for a time control: the estimated duration (base + 40 × increment, in seconds),
/// as Lichess names its speeds. A daily game (`1/86400`) is correspondence.
pub fn speed(time_control: &str, time_class: &str) -> &'static str {
    if time_class == "daily" || time_control.contains('/') {
        return "correspondence";
    }
    let mut parts = time_control.split('+');
    let base: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let increment: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let total = base + 40.0 * increment;
    if total < 30.0 {
        "ultraBullet"
    } else if total < 180.0 {
        "bullet"
    } else if total < 480.0 {
        "blitz"
    } else if total < 1500.0 {
        "rapid"
    } else {
        "classical"
    }
}

/// Lichess's variant key for Chess.com's rules; standard chess is `standard`. None for rules KChess
/// cannot replay (bughouse, odds and the rest), whose games are not kept.
pub fn variant(rules: &str) -> Option<&'static str> {
    Some(match rules {
        "chess" => "standard",
        "chess960" => "chess960",
        "kingofthehill" => "kingOfTheHill",
        "threecheck" => "threeCheck",
        "crazyhouse" => "crazyhouse",
        _ => return None,
    })
}

/// Lichess's status for how a game ended, from the loser's result (or either side's, in a draw).
pub fn status(loser_result: &str) -> &'static str {
    match loser_result {
        "checkmated" => "mate",
        "resigned" => "resign",
        "timeout" => "outoftime",
        "abandoned" => "timeout",
        "stalemate" => "stalemate",
        "kingofthehill" | "threecheck" => "variantEnd",
        _ => "draw",
    }
}

/// The game's start, from its PGN's `UTCDate` and `UTCTime`, in milliseconds.
fn started_at(game: &Game) -> Option<i64> {
    let date = game.header("UTCDate")?;
    let mut day = date.split('.').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (day.next()??, day.next()??, day.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut time = game
        .header("UTCTime")
        .unwrap_or("00:00:00")
        .split(':')
        .map(|part| part.parse::<i64>().ok());
    let (hours, minutes, seconds) = (
        time.next().flatten().unwrap_or(0),
        time.next().flatten().unwrap_or(0),
        time.next().flatten().unwrap_or(0),
    );
    // Days from the civil date (Howard Hinnant's algorithm).
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(((days * 24 + hours) * 60 + minutes) * 60_000 + seconds * 1000)
}

/// The main line's moves, in SAN.
fn main_line(game: &Game) -> Vec<String> {
    let mut sans = Vec::new();
    let mut node = 0;
    while let Some(next) = game.children(node).next() {
        sans.push(game.node(next).san.as_str().to_string());
        node = next;
    }
    sans
}

/// The opening a standard game reaches, named from the bundled list as Lichess names it.
fn opening(start: &str, sans: &[String]) -> Option<String> {
    let fen = rules::parse_fen(start).map(|setup| rules::make_setup_fen(&setup))?;
    let uci = san_to_uci(&fen, sans);
    opening_at("standard", &fen, &uci, uci.len() as f64).map(|named| named.name)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// One game of `username`'s archive as a synced game row (the `games` table's columns), or None
/// when it cannot be kept: rules KChess cannot replay, a game without its player, or no PGN.
pub fn game_row(username: &str, game: &Value) -> Option<Value> {
    let id = game_id(text(game, "url"))?;
    let variant = variant(text(game, "rules"))?;
    let pgn = text(game, "pgn");
    let parsed = parse_pgn(pgn).into_iter().next()?;
    let (white, black) = (game.get("white")?, game.get("black")?);
    let is = |side: &Value| text(side, "username").eq_ignore_ascii_case(username);
    let (color, player, opponent) = if is(white) {
        ("white", white, black)
    } else if is(black) {
        ("black", black, white)
    } else {
        return None;
    };
    let (white_result, black_result) = (text(white, "result"), text(black, "result"));
    let (winner, status) = if white_result == "win" {
        (Some("white"), status(black_result))
    } else if black_result == "win" {
        (Some("black"), status(white_result))
    } else {
        (None, status(white_result))
    };
    let speed = speed(text(game, "time_control"), text(game, "time_class"));
    let perf = if variant == "standard" {
        speed
    } else {
        variant
    };
    let sans = main_line(&parsed);
    let start = parsed.header("FEN").unwrap_or(INITIAL_FEN);
    let opening = (variant == "standard")
        .then(|| opening(start, &sans))
        .flatten();
    let ended_at = game
        .get("end_time")
        .and_then(Value::as_i64)
        .map(|s| s * 1000);
    let started = started_at(&parsed).or(ended_at).unwrap_or(0);
    let rating = |side: &Value| side.get("rating").and_then(Value::as_i64);
    Some(json!({
        "account": account_of(username),
        "id": id,
        "createdAt": started,
        "lastMoveAt": ended_at.unwrap_or(started),
        "rated": game.get("rated").and_then(Value::as_bool).unwrap_or(false),
        "speed": speed,
        "perf": perf,
        "status": status,
        "winner": winner,
        "color": color,
        "opponent": text(opponent, "username"),
        "opponentRating": rating(opponent),
        "playerRating": rating(player),
        "ratingDiff": Value::Null,
        "opening": opening,
        "moves": sans.join(" "),
        "pgn": pgn,
    }))
}

/// A month of the archive list (`.../games/2024/03`), as `2024/03`.
pub fn archive_month(url: &str) -> Option<String> {
    let mut parts = url.trim_end_matches('/').rsplit('/');
    let month = parts.next()?;
    let year = parts.next()?;
    let valid = year.len() == 4
        && month.len() == 2
        && year
            .bytes()
            .chain(month.bytes())
            .all(|b| b.is_ascii_digit());
    valid.then(|| format!("{year}/{month}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PGN: &str = r#"[Event "Live Chess"]
[Site "Chess.com"]
[Date "2024.03.05"]
[White "Alice"]
[Black "bob"]
[Result "1-0"]
[UTCDate "2024.03.05"]
[UTCTime "10:20:30"]
[TimeControl "180+2"]

1. e4 {[%clk 0:03:01]} 1... c5 {[%clk 0:03:00]} 2. Nf3 d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 a6 6. Qh5 1-0"#;

    fn game(rules: &str, white: &str, black: &str, pgn: &str) -> Value {
        json!({
            "url": "https://www.chess.com/game/live/104785391234",
            "pgn": pgn,
            "time_control": "180+2",
            "time_class": "blitz",
            "end_time": 1_709_634_000,
            "rated": true,
            "rules": rules,
            "white": { "username": "Alice", "rating": 1500, "result": white },
            "black": { "username": "bob", "rating": 1450, "result": black },
        })
    }

    #[test]
    fn a_won_game_is_kept_as_lichess_would_keep_it() {
        let row = game_row("alice", &game("chess", "win", "resigned", PGN)).unwrap();
        assert_eq!(row["account"], "alice@chess.com");
        assert_eq!(row["id"], "cc-l104785391234");
        assert_eq!(row["color"], "white");
        assert_eq!(row["opponent"], "bob");
        assert_eq!(row["winner"], "white");
        assert_eq!(row["status"], "resign");
        assert_eq!(row["speed"], "blitz");
        assert_eq!(row["perf"], "blitz");
        assert_eq!(row["playerRating"], 1500);
        assert_eq!(row["opponentRating"], 1450);
        assert_eq!(row["moves"], "e4 c5 Nf3 d6 d4 cxd4 Nxd4 Nf6 Nc3 a6 Qh5");
        assert_eq!(row["createdAt"], 1_709_634_030_000_i64);
        assert_eq!(row["lastMoveAt"], 1_709_634_000_000_i64);
        assert!(
            row["opening"]
                .as_str()
                .unwrap()
                .starts_with("Sicilian Defense"),
            "{}",
            row["opening"]
        );
    }

    #[test]
    fn the_player_is_found_on_either_side_whatever_the_case() {
        let row = game_row("BOB", &game("chess", "checkmated", "win", PGN)).unwrap();
        assert_eq!(row["color"], "black");
        assert_eq!(row["winner"], "black");
        assert_eq!(row["status"], "mate");
        assert_eq!(row["account"], "BOB@chess.com");
        assert!(game_row("carol", &game("chess", "win", "resigned", PGN)).is_none());
    }

    #[test]
    fn every_result_has_lichess_words() {
        for (result, expected) in [
            ("checkmated", "mate"),
            ("resigned", "resign"),
            ("timeout", "outoftime"),
            ("abandoned", "timeout"),
            ("stalemate", "stalemate"),
            ("kingofthehill", "variantEnd"),
            ("threecheck", "variantEnd"),
            ("agreed", "draw"),
            ("repetition", "draw"),
            ("50move", "draw"),
            ("insufficient", "draw"),
            ("timevsinsufficient", "draw"),
        ] {
            assert_eq!(status(result), expected, "{result}");
        }
        let drawn = game_row("alice", &game("chess", "agreed", "agreed", PGN)).unwrap();
        assert_eq!(drawn["winner"], Value::Null);
        assert_eq!(drawn["status"], "draw");
        let stalemate = game_row("alice", &game("chess", "stalemate", "stalemate", PGN)).unwrap();
        assert_eq!(stalemate["status"], "stalemate");
    }

    #[test]
    fn variants_keep_their_rules_and_the_rest_are_skipped() {
        let pgn960 = r#"[Event "Live Chess - Chess960"]
[SetUp "1"]
[FEN "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1"]
[Result "*"]

1. d4 d5 *"#;
        let row = game_row("alice", &game("chess960", "win", "resigned", pgn960)).unwrap();
        assert_eq!(row["perf"], "chess960");
        assert_eq!(row["speed"], "blitz");
        assert_eq!(row["opening"], Value::Null);
        assert!(row["pgn"].as_str().unwrap().contains("[FEN \"bbqnnrkr"));
        for (rules, perf) in [
            ("kingofthehill", "kingOfTheHill"),
            ("threecheck", "threeCheck"),
            ("crazyhouse", "crazyhouse"),
        ] {
            let row = game_row("alice", &game(rules, "win", rules, PGN)).unwrap();
            assert_eq!(row["perf"], perf);
        }
        assert!(game_row("alice", &game("bughouse", "win", "resigned", PGN)).is_none());
        assert!(game_row("alice", &game("oddschess", "win", "resigned", PGN)).is_none());
    }

    #[test]
    fn speeds_follow_lichess_estimate() {
        assert_eq!(speed("15", "bullet"), "ultraBullet");
        assert_eq!(speed("60", "bullet"), "bullet");
        assert_eq!(speed("120+1", "bullet"), "bullet");
        assert_eq!(speed("180", "blitz"), "blitz");
        assert_eq!(speed("300+5", "blitz"), "rapid");
        assert_eq!(speed("600", "rapid"), "rapid");
        assert_eq!(speed("1500", "rapid"), "classical");
        assert_eq!(speed("1800+30", "rapid"), "classical");
        assert_eq!(speed("1/86400", "daily"), "correspondence");
        // Either sign of a daily game is enough.
        assert_eq!(speed("1/86400", ""), "correspondence");
        assert_eq!(speed("600", "daily"), "correspondence");
        // Each bound starts the next speed, as on Lichess.
        assert_eq!(speed("29", "bullet"), "ultraBullet");
        assert_eq!(speed("30", "bullet"), "bullet");
        assert_eq!(speed("179", "bullet"), "bullet");
        assert_eq!(speed("180", "blitz"), "blitz");
        assert_eq!(speed("479", "blitz"), "blitz");
        assert_eq!(speed("480", "blitz"), "rapid");
        assert_eq!(speed("1499", "rapid"), "rapid");
    }

    #[test]
    fn a_game_starts_at_its_utc_date_and_time() {
        let created_at = |date: &str, time: &str| {
            let pgn = PGN
                .replace("2024.03.05\"]\n[UTCTime", &format!("{date}\"]\n[UTCTime"))
                .replace("10:20:30", time);
            game_row("alice", &game("chess", "win", "resigned", &pgn)).unwrap()["createdAt"].clone()
        };
        // Expected values from Python's `datetime`; January, February, leap days, century
        // years and every clock field each take their own part of the conversion.
        for (date, time, millis) in [
            ("2024.02.29", "23:59:59", 1_709_251_199_000_i64),
            ("2000.01.01", "00:00:00", 946_684_800_000),
            ("1999.12.31", "12:34:56", 946_643_696_000),
            ("2100.03.01", "00:00:00", 4_107_542_400_000),
            ("2023.01.15", "08:05:09", 1_673_769_909_000),
        ] {
            assert_eq!(created_at(date, time), millis, "{date} {time}");
        }
        // A date that is no date falls back to the game's end.
        for date in [
            "2024.13.05",
            "2024.00.05",
            "2024.03.32",
            "2024.03.00",
            "2024.03",
        ] {
            assert_eq!(
                created_at(date, "10:20:30"),
                1_709_634_000_000_i64,
                "{date}"
            );
        }
    }

    #[test]
    fn a_daily_game_is_correspondence_with_its_own_ids() {
        let mut daily = game("chess", "win", "resigned", PGN);
        daily["url"] = json!("https://www.chess.com/game/daily/987654");
        daily["time_control"] = json!("1/259200");
        daily["time_class"] = json!("daily");
        let row = game_row("alice", &daily).unwrap();
        assert_eq!(row["id"], "cc-d987654");
        assert_eq!(row["speed"], "correspondence");
        assert_eq!(row["perf"], "correspondence");
    }

    #[test]
    fn ids_accounts_and_months_are_strict() {
        assert!(is_game_id("cc-l1"));
        assert!(is_game_id("cc-d12345678901234567890"));
        assert!(!is_game_id("cc-x1"));
        assert!(!is_game_id("cc-l"));
        assert!(!is_game_id("cc-l12a"));
        assert!(!is_game_id("abcd1234"));
        assert_eq!(
            game_id("https://www.chess.com/game/live/42/"),
            Some("cc-l42".into())
        );
        assert_eq!(
            game_id("https://www.chess.com/analysis/game/live/42"),
            Some("cc-l42".into())
        );
        assert_eq!(game_id("https://www.chess.com/game/computer/42"), None);
        assert_eq!(username_of("Hikaru@chess.com"), Some("Hikaru"));
        assert_eq!(username_of("Hikaru@Chess.com"), Some("Hikaru"));
        assert_eq!(username_of("Hikaru"), None);
        assert_eq!(username_of("ab@chess.com"), None);
        assert_eq!(username_of("a b@chess.com"), None);
        assert!(is_username("magnus_carlsen-1"));
        assert!(!is_username("ab"));
        assert_eq!(
            archive_month("https://api.chess.com/pub/player/a/games/2024/03"),
            Some("2024/03".into())
        );
        assert_eq!(
            archive_month("https://api.chess.com/pub/player/a/games"),
            None
        );
        for url in [
            "https://api.chess.com/pub/player/a/games/24/03",
            "https://api.chess.com/pub/player/a/games/2024/3",
            "https://api.chess.com/pub/player/a/games/2024/0x",
        ] {
            assert_eq!(archive_month(url), None, "{url}");
        }
    }
}
