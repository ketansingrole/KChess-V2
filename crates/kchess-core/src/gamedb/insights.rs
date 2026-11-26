//! Insights over a game database's player (`gameDbInsights`): the player's games in one database
//! made into the rows, replays, clocks and reviews `store::insights` makes an account's report
//! from, so both reports mean the same.
//!
//! What a synced game has as columns, a game of a database has in its headers: its speed comes
//! from `TimeControl`, when it was played from `UTCDate` and `UTCTime` (its date alone gives its
//! day and no hour), whether it was rated from Lichess's event name ("Rated blitz game") or the
//! download's request, and how it ended from its last position and `Termination`. The player's
//! mistakes come from the `%eval`s of the games Lichess analysed (`review::pgn_review`), run
//! through the same analysis as a synced game's review; nothing runs an engine.

use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, params_from_iter, types::Value as Sql};
use shakmaty::variant::VariantPosition;
use shakmaty::{Chess, Color, Move, Position as _};
use shakmaty_syzygy::Tablebase;

use super::{report, search};
use crate::host::{Host, Level};
use crate::store::insights::{Clocks, DAY_MS, Mistakes, ROW_LIMIT, Replays, Row, results};
use crate::store::reviews::analyse_stored;
use crate::store::sql::Sql as _;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{InsightsQuery, InsightsReport};
use kchess_domain::calendar::days_from_civil;
use kchess_domain::gamedb::{self, Outcome, Stored, patterns};
use kchess_domain::review::pgn_review;
use kchess_domain::time_use::time_use_of;
use kchess_domain::variant::Variant;

const DAY: i64 = DAY_MS as i64;

/// One of the player's games as the first read keeps it: for the report's order, and for the
/// second read, which replays it and reads its notes.
struct Kept {
    id: i64,
    /// When it was played (UTC milliseconds), or its day's start when only its date is known.
    at: Option<i64>,
    you: Color,
    /// Whether it has notes, where its clocks and evals are when it has any.
    notes: bool,
}

/// The report of the player `query.account` (in any case) in `db`, of the games `query`'s
/// filters keep: a speed only of games whose time control is known, rated or casual only of
/// games whose rating is known (every one is rated when the download asked for rated games
/// alone, `rated_download`), a period only of dated games. A variant has no games: a database
/// holds standard chess. The newest `ROW_LIMIT` games are counted, and of those the newest are
/// replayed, and their clocks and evals read, within the limits an account's are.
pub fn insights(
    host: &dyn Host,
    db: &Connection,
    query: &InsightsQuery,
    rated_download: bool,
    now: f64,
    tables: Option<&Tablebase<Chess>>,
    stop: &AtomicBool,
) -> Result<InsightsReport> {
    if query
        .variant
        .is_some_and(|variant| variant != Variant::Standard)
    {
        return results(&query.account, &[]);
    }
    let games = read(db, query, rated_download, now, stop)?;
    let (kept, rows): (Vec<Kept>, Vec<Row>) = games.into_iter().unzip();
    let mut report = results(&query.account, &rows)?;
    let mut replays = Replays::default();
    let mut clocks = Clocks::default();
    let mut mistakes = Mistakes::default();
    for (game, row) in kept.iter().zip(&rows) {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("Insights stopped."));
        }
        let replay = !replays.full();
        let notes = game.notes && !(clocks.full() && mistakes.full());
        if !replay && !notes {
            if clocks.full() && mistakes.full() {
                break;
            }
            continue;
        }
        let Some(stored) = search::stored(db, game.id as u64)? else {
            continue;
        };
        if replay {
            replays.add(game.you, &row.result, replayed(&stored), tables)?;
        }
        if !notes {
            continue;
        }
        let Some(pgn) = gamedb::game_of(&stored) else {
            continue;
        };
        // A review that cannot be analysed leaves the game unreviewed.
        let analysis = pgn_review(&pgn).and_then(|review| match analyse_stored(&review) {
            Ok(analysis) => Some(analysis),
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "insights",
                    &format!("A player's game's evals left out: {cause}"),
                );
                None
            }
        });
        if let Some(analysis) = &analysis
            && !mistakes.full()
        {
            let side = analysis.side(game.you);
            mistakes.reviewed();
            mistakes.side(
                side.accuracy.as_ref().and_then(serde_json::Number::as_f64),
                side.acpl.as_ref().and_then(serde_json::Number::as_f64),
                [side.inaccuracy, side.mistake, side.blunder].map(f64::from),
            );
        }
        if !clocks.full()
            && let Some(time) = time_use_of(&pgn)
        {
            let verdicts = analysis.as_ref().map(|analysis| analysis.moves.as_slice());
            clocks.add(game.you == Color::White, &row.result, &time, verdicts);
        }
    }
    report.accuracy = mistakes.report();
    report.time_use = clocks.finish();
    Ok(with_replays(report, replays, tables))
}

/// `report` with the endgames and structures of `replays`.
fn with_replays(
    mut report: InsightsReport,
    replays: Replays,
    tables: Option<&Tablebase<Chess>>,
) -> InsightsReport {
    (report.endgames, report.structures) = replays.report(tables.is_some());
    report
}

/// The player's finished games that `query` keeps, the newest `ROW_LIMIT`, newest first.
fn read(
    db: &Connection,
    query: &InsightsQuery,
    rated_download: bool,
    now: f64,
    stop: &AtomicBool,
) -> Result<Vec<(Kept, Row)>> {
    let mut ids: Vec<i64> = db
        .prepare("SELECT id FROM players WHERE lower(trim(name)) = ?1")
        .sql()?
        .query_map([query.account.trim().to_lowercase()], |row| row.get(0))
        .sql()?
        .collect::<rusqlite::Result<_>>()
        .sql()?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    ids.sort_unstable();
    let marks = vec!["?"; ids.len()].join(", ");
    let mut params: Vec<Sql> = ids.iter().map(|id| Sql::Integer(*id)).collect();
    params.extend(ids.iter().map(|id| Sql::Integer(*id)));
    params.push(Sql::Integer(search::visible(db)?));
    let mut statement = db
        .prepare(&format!(
            "SELECT {}, g.id, g.white, g.date_key FROM games g {}
             WHERE (g.white IN ({marks}) OR g.black IN ({marks})) AND g.import_id <= ?
             ORDER BY g.date_key DESC, g.id DESC",
            search::STORED_COLUMNS,
            search::JOINS
        ))
        .sql()?;
    // The local day and hour as an account's report has them (`store::insights`).
    let mut local = db
        .prepare("SELECT strftime('%w %H', ?1 / 1000, 'unixepoch', 'localtime')")
        .sql()?;
    let since = query
        .days
        .filter(|days| *days > 0)
        .map(|days| now - days as f64 * DAY_MS);
    let mut rows = statement.query(params_from_iter(params.iter())).sql()?;
    let mut games: Vec<(Kept, Row)> = Vec::new();
    let mut last_day = i64::MAX;
    while let Some(row) = rows.next().sql()? {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("Insights stopped."));
        }
        let date_key: i64 = row.get(18).sql()?;
        // The games come by day, newest first: once enough are in, a game of an earlier day is
        // older than every one kept.
        if games.len() as i64 >= ROW_LIMIT && date_key < last_day {
            break;
        }
        let stored = search::stored_row(row).sql()?;
        let id: i64 = row.get(16).sql()?;
        let you = if ids.binary_search(&row.get::<_, i64>(17).sql()?).is_ok() {
            Color::White
        } else {
            Color::Black
        };
        let Some(result) = result_for(stored.outcome, you) else {
            continue;
        };
        let header = |name: &str| gamedb::stored_header(&stored, name);
        let speed = header("TimeControl")
            .as_deref()
            .and_then(patterns::speed)
            .map(patterns::Speed::key);
        if query
            .speed
            .is_some_and(|wanted| speed != Some(wanted.as_str()))
        {
            continue;
        }
        if let Some(wanted) = query.rated
            && rated(stored.event.as_deref(), rated_download) != Some(wanted)
        {
            continue;
        }
        let date = header("UTCDate").or_else(|| stored.date.clone());
        let day = date.as_deref().and_then(day_of);
        let timed = day
            .zip(header("UTCTime").as_deref().and_then(seconds_of))
            .map(|(day, seconds)| day * DAY + seconds * 1000);
        let at = timed.or(day.map(|day| day * DAY));
        if let Some(since) = since
            && at.is_none_or(|at| (at as f64) < since)
        {
            continue;
        }
        let (weekday, hour) = match (timed, day) {
            (Some(at), _) => {
                let text: Option<String> = local.query_row([at as f64], |row| row.get(0)).sql()?;
                text.as_deref()
                    .and_then(|text| text.split_once(' '))
                    .map_or((-1, -1), |(day, hour)| {
                        (
                            day.parse::<i64>().unwrap_or(-1),
                            hour.parse::<i64>().unwrap_or(-1),
                        )
                    })
            }
            // 1 January 1970, day 0, was a Thursday.
            (None, Some(day)) => ((day + 4).rem_euclid(7), -1),
            (None, None) => (-1, -1),
        };
        let elo = |color: Color| match color {
            Color::White => stored.white_elo,
            Color::Black => stored.black_elo,
        };
        let opening = report::opening_name(
            stored.opening.as_deref(),
            stored.eco.as_deref(),
            &report::first_moves(stored.fen.as_deref(), &stored.moves),
        );
        let row = Row {
            color: if you == Color::White {
                "white"
            } else {
                "black"
            }
            .to_string(),
            speed: speed.map(str::to_string),
            status: ending(&stored, header("Termination").as_deref()).to_string(),
            opening: (opening != "Other").then_some(opening),
            opponent_rating: elo(!you).map(f64::from),
            player_rating: elo(you).map(f64::from),
            result: result.to_string(),
            weekday,
            hour,
            plies: stored.plies as i64,
        };
        let kept = Kept {
            id,
            at,
            you,
            notes: stored.extra.is_some(),
        };
        games.push((kept, row));
        last_day = date_key;
    }
    // The newest first, to the second where the games say when they were played; games of one
    // moment in the order the database has them.
    games.sort_by(|(a, _), (b, _)| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    games.truncate(ROW_LIMIT as usize);
    Ok(games)
}

/// `win`, `loss` or `draw` for the player with `you`; none for an unknown result.
fn result_for(outcome: Outcome, you: Color) -> Option<&'static str> {
    Some(match (outcome, you) {
        (Outcome::Draw, _) => "draw",
        (Outcome::White, Color::White) | (Outcome::Black, Color::Black) => "win",
        (Outcome::White, Color::Black) | (Outcome::Black, Color::White) => "loss",
        (Outcome::Unknown, _) => return None,
    })
}

/// Whether a game was rated: Lichess names its events "Rated blitz game" or "Casual blitz game";
/// any other is rated when the download asked for rated games only, and unknown otherwise.
fn rated(event: Option<&str>, rated_download: bool) -> Option<bool> {
    let event = event.unwrap_or_default().to_lowercase();
    if event.starts_with("rated ") || rated_download {
        Some(true)
    } else if event.starts_with("casual ") {
        Some(false)
    } else {
        None
    }
}

/// The days since 1970 of a PGN date, `YYYY.MM.DD`; none unless all three are known.
fn day_of(date: &str) -> Option<i64> {
    let mut parts = date
        .trim()
        .split(['.', '-', '/'])
        .map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1000..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    days_from_civil(year, month, day)
}

/// The seconds into its day of a PGN time, `HH:MM:SS`.
fn seconds_of(time: &str) -> Option<i64> {
    let mut parts = time.trim().split(':').map(|part| part.parse::<i64>().ok());
    let (hours, minutes, seconds) = (parts.next()??, parts.next()??, parts.next()??);
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) || !(0..=60).contains(&seconds) {
        return None;
    }
    Some((hours * 60 + minutes) * 60 + seconds)
}

/// How a game ended, as Lichess's status names it: checkmate and stalemate from its last
/// position, a game lost on time or left from its `Termination` (Lichess's "Time forfeit" and
/// "Abandoned", Chess.com's "won on time" and "game abandoned"), else a draw or a resignation.
fn ending(stored: &Stored, termination: Option<&str>) -> &'static str {
    let last = gamedb::start_of(stored.fen.as_deref()).and_then(|start| {
        let moves = gamedb::decode_line(&start, &stored.moves)?;
        let mut pos = start;
        for played in moves {
            pos.play_unchecked(played);
        }
        Some(pos)
    });
    if last.as_ref().is_some_and(VariantPosition::is_checkmate) {
        return "mate";
    }
    if last.as_ref().is_some_and(VariantPosition::is_stalemate) {
        return "stalemate";
    }
    let termination = termination.unwrap_or_default().to_lowercase();
    let decisive = stored.outcome != Outcome::Draw;
    if termination.contains("abandon") {
        "timeout"
    } else if termination.contains("infraction") {
        "cheat"
    } else if decisive && termination.contains("time") {
        "outoftime"
    } else if decisive {
        "resign"
    } else {
        "draw"
    }
}

/// A stored game's start and moves, as the replay of an account's games takes them; none for a
/// game that does not replay.
fn replayed(stored: &Stored) -> Option<(Chess, Vec<Move>)> {
    let start = gamedb::start_of(stored.fen.as_deref())?;
    let moves = gamedb::decode_line(&start, &stored.moves)?;
    match start {
        VariantPosition::Chess(pos) => Some((pos, moves)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::gamedb::{GameDbService, schema, write_database};
    use crate::host::Config;
    use crate::test_host::TestHost;
    use kchess_contracts::types::{
        ExplorerSpeed, GameRecord, GameSite, InsightsFilters, PlayerGames,
    };

    /// A player's games as Lichess and Chess.com export them: Scout loses a rated blitz game
    /// Lichess analysed, letting a mate in one in short of time; wins a casual rapid game on
    /// resignation; draws a Chess.com blitz game (named "scout" there, with Chess.com's
    /// clocks); and wins one with only a date. A game of others and an unfinished one are no
    /// results of Scout's.
    const SCOUT: &str = "[Event \"Rated blitz game\"]\n[Site \"https://lichess.org/aaaaaaaa\"]\n\
        [Date \"2026.11.01\"]\n[White \"Rival\"]\n[Black \"Scout\"]\n[Result \"1-0\"]\n\
        [UTCDate \"2026.11.01\"]\n[UTCTime \"10:00:00\"]\n[WhiteElo \"1900\"]\n\
        [BlackElo \"2000\"]\n[TimeControl \"180+2\"]\n[Termination \"Normal\"]\n\
        [Opening \"King's Pawn Game: Wayward Queen Attack\"]\n\n\
        1. e4 { [%eval 0.3] [%clk 0:03:00] } 1... e5 { [%eval 0.3] [%clk 0:03:00] } \
        2. Qh5 { [%eval 0.0] [%clk 0:02:58] } 2... Nc6 { [%eval 0.1] [%clk 0:02:55] } \
        3. Bc4 { [%eval 0.2] [%clk 0:02:50] } 3... Nf6 { [%eval #1] [%clk 0:00:10] } \
        4. Qxf7# { [%clk 0:02:45] } 1-0\n\n\
        [Event \"Casual rapid game\"]\n[Date \"2026.11.02\"]\n[White \"Scout\"]\n\
        [Black \"Rival2\"]\n[Result \"1-0\"]\n[UTCDate \"2026.11.02\"]\n[UTCTime \"12:00:00\"]\n\
        [WhiteElo \"2000\"]\n[BlackElo \"2250\"]\n[TimeControl \"600+0\"]\n\
        [Termination \"Normal\"]\n\n\
        1. d4 { [%clk 0:10:00] } 1... d5 { [%clk 0:10:00] } 2. c4 { [%clk 0:09:50] } \
        2... e6 { [%clk 0:09:40] } 3. Nc3 { [%clk 0:09:30] } 3... Nf6 { [%clk 0:09:00] } 1-0\n\n\
        [Event \"Live Chess\"]\n[Site \"Chess.com\"]\n[Date \"2026.11.03\"]\n\
        [White \"Rival3\"]\n[Black \"scout\"]\n[Result \"1/2-1/2\"]\n[UTCDate \"2026.11.03\"]\n\
        [UTCTime \"09:00:00\"]\n[WhiteElo \"2010\"]\n[BlackElo \"2000\"]\n\
        [TimeControl \"180\"]\n[Termination \"Game drawn by agreement\"]\n\
        [Opening \"King's Pawn Game\"]\n\n\
        1. e4 {[%clk 0:02:59.9]} 1... c5 {[%clk 0:02:58.1]} 2. Nf3 {[%clk 0:02:55]} \
        2... d6 {[%clk 0:02:50]} 1/2-1/2\n\n\
        [Event \"?\"]\n[Date \"2026.10.30\"]\n[White \"Rival4\"]\n[Black \"Scout\"]\n\
        [Result \"0-1\"]\n\n1. f3 e5 2. g4 d5 0-1\n\n\
        [White \"Other\"]\n[Black \"Rival\"]\n[Result \"1-0\"]\n\n1. e4 1-0\n\n\
        [White \"Scout\"]\n[Black \"Rival\"]\n[Result \"*\"]\n\n1. e4 *\n";

    /// 2026-11-03T12:00:00Z.
    const NOW: f64 = 1_793_707_200_000.0;

    fn games(win: i64, draw: i64, loss: i64) -> GameRecord {
        GameRecord {
            total: win + draw + loss,
            win,
            draw,
            loss,
        }
    }

    fn report_of(profile: &Path, id: &str, filters: InsightsFilters) -> InsightsReport {
        let db = schema::open_read(&profile.join("databases").join(format!("{id}.kdb"))).unwrap();
        let host = TestHost::quiet();
        insights(
            &*host,
            &db,
            &filters.of("Scout"),
            false,
            NOW,
            None,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    /// Scout's report of `SCOUT`, unfiltered.
    fn scout_report() -> InsightsReport {
        let profile = tempfile::tempdir().unwrap();
        let id = write_database(profile.path(), "Scout", SCOUT).unwrap();
        report_of(profile.path(), &id, InsightsFilters::default())
    }

    /// The player's games give the results an account's games give: their record, by colour,
    /// speed, opponent, length, opening and ending, the streaks oldest first and the day of a
    /// game with only a date.
    #[test]
    fn a_players_games_are_their_insights() {
        let report = scout_report();
        assert_eq!(report.account, "Scout");
        assert_eq!(report.total, 4);
        assert_eq!(report.record, games(2, 1, 1));
        assert_eq!(report.by_color.white, games(1, 0, 0));
        assert_eq!(report.by_color.black, games(1, 1, 1));
        let speeds: Vec<(&str, GameRecord)> = report
            .by_speed
            .iter()
            .map(|entry| (entry.speed.as_str(), entry.record))
            .collect();
        assert_eq!(
            speeds,
            [("blitz", games(0, 1, 1)), ("rapid", games(1, 0, 0))],
            "the game without a time control has no speed"
        );
        let band = |label: &str| {
            report
                .by_opponent
                .iter()
                .find(|band| band.label.starts_with(label))
                .map(|band| band.record)
        };
        assert_eq!(band("Weaker"), Some(games(0, 0, 1)));
        assert_eq!(band("Similar"), Some(games(0, 1, 0)));
        assert_eq!(band("Much stronger"), Some(games(1, 0, 0)));
        assert_eq!(report.by_length[0].record, games(2, 1, 1));
        let endings: Vec<(&str, i64)> = report
            .endings
            .iter()
            .map(|entry| (entry.status.as_str(), entry.record.total))
            .collect();
        assert_eq!(endings, [("resign", 2), ("mate", 1), ("draw", 1)]);
        // Oldest first: a win, a loss, a win and the draw last.
        assert_eq!(
            (report.streaks.longest_win, report.streaks.longest_loss),
            (1, 1)
        );
        assert_eq!(report.streaks.current, 0);
        // 30 October 2026 was a Friday; its game has no hour.
        assert_eq!(report.by_weekday[5].win, 1);
        let hours: i64 = report.by_hour.iter().map(|hour| hour.total).sum();
        assert_eq!(hours, 3);
        assert_eq!(report.by_opening.len(), 1);
        assert_eq!(report.by_opening[0].name, "King's Pawn Game");
        assert_eq!(report.by_opening[0].record, games(0, 1, 1));
        assert!(report.variants.is_empty());
    }

    /// Replaying the player's games adds what an account's do: their time use from either site's
    /// clocks, their mistakes from the evals of the game Lichess analysed, and their endgames
    /// and pawn structures.
    #[test]
    fn a_players_replayed_games_are_their_insights() {
        let report = scout_report();
        let accuracy = report.accuracy.expect("the analysed game's accuracy");
        assert_eq!(accuracy.games, 1);
        assert_eq!(accuracy.per_game.blunder, 1.0);
        assert!(accuracy.average.is_some() && accuracy.acpl.is_some());

        let time = report.time_use.expect("time use from the clocks");
        assert_eq!(time.games, 3, "both sites' clocked games");
        assert_eq!(time.trouble, games(0, 0, 1), "10 s left after the blunder");
        assert_eq!(time.spare, games(1, 1, 0));
        let blunders = time.blunders.expect("the analysed game's blunders");
        assert_eq!(blunders.games, 1);
        assert_eq!((blunders.spare.moves, blunders.spare.blunders), (3, 1));

        let endgames = report.endgames.expect("endgames, undecided without tables");
        assert!(!endgames.tables);
        assert_eq!(
            report.structures.map(|structures| structures.games),
            Some(4)
        );
    }

    /// The filters keep the games that say they match: a speed of games with a time control,
    /// rated or casual of Lichess's events, a period of dated games; a variant has none.
    #[test]
    fn a_players_insights_filter_what_their_games_tell() {
        let profile = tempfile::tempdir().unwrap();
        let id = write_database(profile.path(), "Scout", SCOUT).unwrap();
        let total = |filters: InsightsFilters| report_of(profile.path(), &id, filters).total;
        let blitz = InsightsFilters {
            speed: Some(ExplorerSpeed::Blitz),
            ..InsightsFilters::default()
        };
        assert_eq!(total(blitz), 2);
        let rated = |rated| InsightsFilters {
            rated: Some(rated),
            ..InsightsFilters::default()
        };
        assert_eq!(
            total(rated(true)),
            1,
            "Chess.com's game and the one with only a date may be either"
        );
        assert_eq!(total(rated(false)), 1);
        // The two days before noon on 3 November: from noon on 1 November.
        let days = InsightsFilters {
            days: Some(2),
            ..InsightsFilters::default()
        };
        assert_eq!(total(days), 2);
        let variant = InsightsFilters {
            variant: Some(Variant::Chess960),
            ..InsightsFilters::default()
        };
        let report = report_of(profile.path(), &id, variant);
        assert_eq!(report.total, 0);
        assert!(report.endgames.is_none() && report.structures.is_none());
    }

    /// A downloaded player's database answers their insights through the service, every game
    /// rated when the download asked for rated games only; a database of no player is refused.
    #[tokio::test]
    async fn a_downloaded_players_insights_come_from_their_database() {
        let (profile, empty) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let host = TestHost::new();
        let service = GameDbService::new(&Config::new(profile.path().to_path_buf()), host);
        let tables = crate::tablebase::Tablebases::new(
            TestHost::quiet(),
            Arc::new(crate::usage::Unmetered),
            empty.path().join("missing"),
            String::new(),
            Vec::new(),
        );
        let request = PlayerGames {
            site: GameSite::Lichess,
            username: "Scout".to_string(),
            months: None,
            speeds: None,
            rated: true,
        };
        let installed = service
            .player_install(request, |mut sink| async move {
                sink.pgn(SCOUT)?;
                Ok(sink)
            })
            .await
            .unwrap();
        let rated = InsightsFilters {
            rated: Some(true),
            ..InsightsFilters::default()
        };
        let report = service
            .insights(installed.database.id, rated, tables.clone())
            .await
            .unwrap();
        assert_eq!(report.account, "Scout");
        assert_eq!(report.total, 4, "every game of a rated download is rated");
        let blunders = report.accuracy.map(|accuracy| accuracy.per_game.blunder);
        assert_eq!(blunders, Some(1.0));

        let other = write_database(profile.path(), "Others", SCOUT).unwrap();
        let refused = service
            .insights(other, InsightsFilters::default(), tables)
            .await
            .unwrap_err();
        assert_eq!(refused.message, "Choose a player's games.");
    }
}
