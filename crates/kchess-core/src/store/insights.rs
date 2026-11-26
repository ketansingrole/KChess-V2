//! Patterns in the games synced to this computer: results by
//! colour, speed, opening, time of day, opponent strength and game length, accuracy where
//! games were reviewed, time use where they have clocks, and, from replaying the games, how
//! the endgames the tablebases decide went and the pawn structures reached
//! (`replayed_games`). Everything is read locally; nothing
//! is sent to Lichess.
//!
//! The report is made from rows, one a game (`Row`, `results`), and from what replaying,
//! clocks and reviews add (`Replays`, `Clocks`, `Mistakes`): the synced games' SQL here makes
//! them for an account, and `gamedb::insights` for a player of a game database, so both reports
//! mean the same.

use rusqlite::{Connection, params_from_iter, types::Value as Sql};
use serde_json::Value;

use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::{CastlingMode, Chess, Color, Move, Position as _};
use shakmaty_syzygy::Tablebase;

use super::sql::Sql as _;
use crate::gamedb::endgames;
use crate::host::{Host, Level};
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    AccuracyReport, BandRecord, Blunders, BlundersByTime, ByColor, ClockAt, EndgameRecord,
    EndgameReport, EndingRecord, GameDbStructure, GameRecord, InsightsQuery, InsightsReport,
    Judgment, MistakeRates, OpeningRecord, PhaseTime, SpeedRecord, StoredReview, Streaks,
    StructureRecord, StructureReport, TimeUseReport,
};
use kchess_domain::gamedb::patterns;
use kchess_domain::js;
use kchess_domain::js::truthy;
use kchess_domain::review::MoveVerdict;
use kchess_domain::time_use::{PHASES, TimeUse, time_use};
use kchess_domain::variant::{VARIANTS, Variant};

const NEG_INF: f64 = f64::NEG_INFINITY;
const INF: f64 = f64::INFINITY;

/// Rating difference (opponent minus player) bands, with their labels.
const RATING_BANDS: [(&str, f64, f64); 5] = [
    ("Much weaker (−200 or less)", NEG_INF, -200.0),
    ("Weaker (−199 to −50)", -199.0, -50.0),
    ("Similar (±49)", -49.0, 49.0),
    ("Stronger (+50 to +199)", 50.0, 199.0),
    ("Much stronger (+200 or more)", 200.0, INF),
];

/// Game length bands in plies, with their labels.
const LENGTH_BANDS: [(&str, f64, f64); 4] = [
    ("Under 20 moves", 0.0, 39.0),
    ("20–39 moves", 40.0, 79.0),
    ("40–59 moves", 80.0, 119.0),
    ("60 moves or more", 120.0, INF),
];

pub(crate) const DAY_MS: f64 = 86_400_000.0;
pub(crate) const ROW_LIMIT: i64 = 20_000;
pub(crate) const REVIEW_LIMIT: i64 = 2_000;
/// The newest clocked games read for time use: each one's PGN is parsed and replayed.
pub(crate) const CLOCK_LIMIT: i64 = 2_000;
/// The newest games replayed for their endgames and pawn structures.
pub(crate) const REPLAY_LIMIT: i64 = 2_000;
/// The kinds of endgame listed, won and drawn each.
const ENDGAME_LINES: usize = 8;
/// The moves whose clocks are compared with the opponent's.
const CLOCK_MOVES: [usize; 2] = [20, 40];

/// Counts one game's `result` (`win`, `loss` or `draw`).
fn add(record: &mut GameRecord, result: &str) {
    record.total += 1;
    match result {
        "win" => record.win += 1,
        "loss" => record.loss += 1,
        _ => record.draw += 1,
    }
}

/// First-seen ordered entries with a hash index.
/// A linear scan here made grouping quadratic in the number of distinct speeds, openings or
/// endings (20k distinct opening names took hundreds of milliseconds).
struct Slots<T> {
    items: Vec<(String, T)>,
    index: std::collections::HashMap<String, usize>,
}

impl<T> Slots<T> {
    fn new() -> Self {
        Slots {
            items: Vec::new(),
            index: std::collections::HashMap::new(),
        }
    }

    fn slot(&mut self, key: &str, make: impl FnOnce() -> T) -> &mut T {
        if let Some(&index) = self.index.get(key) {
            return &mut self.items[index].1;
        }
        let index = self.items.len();
        self.items.push((key.to_string(), make()));
        self.index.insert(key.to_string(), index);
        &mut self.items[index].1
    }

    fn into_items(self) -> Vec<(String, T)> {
        self.items
    }
}

/// The filter clauses on `games`, optionally qualified (`g.`) for the review join.
fn clauses(query: &InsightsQuery, prefix: &str, now: f64) -> Result<(Vec<String>, Vec<Sql>)> {
    let mut clauses = vec![format!("{prefix}account = ?")];
    let mut params = vec![Sql::Text(query.account.clone())];
    if let Some(speed) = query.speed {
        clauses.push(format!("{prefix}speed = ?"));
        params.push(Sql::Text(speed.as_str().to_string()));
    }
    if let Some(rated) = query.rated {
        clauses.push(format!("{prefix}rated = ?"));
        params.push(Sql::Integer(i64::from(rated)));
    }
    if let Some(days) = query.days.filter(|days| *days > 0) {
        clauses.push(format!("{prefix}createdAt >= ?"));
        params.push(Sql::Real(now - days as f64 * DAY_MS));
    }
    // Lichess files a variant game under the variant (`perf`): standard chess, by default, leaves
    // them out, so their results and openings do not mix into it.
    match query.variant.map(Variant::key) {
        Some(variant) if variant != "standard" => {
            clauses.push(format!("{prefix}perf = ?"));
            params.push(Sql::Text(variant.to_string()));
        }
        _ => {
            let others = &VARIANTS[1..];
            let marks = vec!["?"; others.len()].join(", ");
            clauses.push(format!("{prefix}perf NOT IN ({marks})"));
            params.extend(others.iter().map(|key| Sql::Text((*key).to_string())));
        }
    }
    // In-progress games have no result yet.
    clauses.push(format!("{prefix}status NOT IN ('created', 'started')"));
    Ok((clauses, params))
}

/// The variants other than standard the account has finished games in, in KChess's order.
fn played_variants(db: &Connection, query: &InsightsQuery) -> Result<Vec<Variant>> {
    let mut statement = db
        .prepare(
            "SELECT DISTINCT perf FROM games
             WHERE account = ? AND status NOT IN ('created', 'started')",
        )
        .sql()?;
    let played: Vec<String> = statement
        .query_map([&query.account], |row| row.get(0))
        .sql()?
        .collect::<std::result::Result<_, _>>()
        .sql()?;
    Ok(VARIANTS[1..]
        .iter()
        .copied()
        .filter(|key| played.iter().any(|perf| perf == key))
        .filter_map(Variant::from_key)
        .collect())
}

/// `insights(query)`.
pub fn insights(
    host: &dyn Host,
    db: &Connection,
    query: &InsightsQuery,
    now: f64,
) -> Result<InsightsReport> {
    let (clauses, params) = clauses(query, "", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
          (CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END) AS result,
          strftime('%w %H', createdAt / 1000, 'unixepoch', 'localtime') AS dayhour,
          CASE WHEN moves = '' THEN 0 ELSE LENGTH(moves) - LENGTH(REPLACE(moves, ' ', '')) + 1 END AS plies
         FROM games WHERE {where_sql} ORDER BY createdAt DESC LIMIT {ROW_LIMIT}"
    );
    let mut statement = db.prepare(&sql).sql()?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            let dayhour: Option<String> = row.get(9)?;
            let (weekday, hour) = dayhour
                .as_deref()
                .and_then(|text| text.split_once(' '))
                .map_or((-1, -1), |(day, hour)| {
                    (
                        day.parse::<i64>().unwrap_or(-1),
                        hour.parse::<i64>().unwrap_or(-1),
                    )
                });
            Ok(Row {
                color: row.get(1)?,
                speed: Some(row.get(2)?),
                opening: row.get(4)?,
                opponent_rating: row.get(5)?,
                player_rating: row.get(6)?,
                result: row.get(8)?,
                weekday,
                hour,
                plies: row.get(10)?,
                status: row.get(3)?,
            })
        })
        .sql()?;
    let mut rows_out = Vec::new();
    for row in rows {
        rows_out.push(row.sql()?);
    }
    let mut report = results(&query.account, &rows_out)?;
    report.accuracy = accuracy_report(host, db, query, now)?;
    report.time_use = time_use_report(host, db, query, now)?;
    report.variants = played_variants(db, query)?;
    Ok(report)
}

/// The report of `account`'s games `rows`, newest first: their results overall and broken
/// down, and the streaks. Accuracy, time use, endgames, structures and variants are the
/// producer's to add.
pub(crate) fn results(account: &str, rows: &[Row]) -> Result<InsightsReport> {
    let mut record = GameRecord::default();
    let mut by_color = [GameRecord::default(), GameRecord::default()];
    let mut speeds: Slots<GameRecord> = Slots::new();
    let mut openings: Slots<(GameRecord, i64)> = Slots::new();
    let mut weekdays: Vec<GameRecord> = (0..7).map(|_| GameRecord::default()).collect();
    let mut hours: Vec<GameRecord> = (0..24).map(|_| GameRecord::default()).collect();
    let mut opponents: Vec<GameRecord> =
        RATING_BANDS.iter().map(|_| GameRecord::default()).collect();
    let mut lengths: Vec<GameRecord> = LENGTH_BANDS.iter().map(|_| GameRecord::default()).collect();
    let mut endings: Slots<GameRecord> = Slots::new();
    let mut longest_win = 0;
    let mut longest_loss = 0;
    let mut run = 0;
    let mut run_kind = String::new();
    // Rows are newest first; streaks are counted oldest first.
    for row in rows.iter().rev() {
        add(&mut record, &row.result);
        let side = match row.color.as_str() {
            "white" => 0,
            "black" => 1,
            _ => {
                return Err(CoreError::new(format!(
                    "A saved game was played as \"{}\", which is neither white nor black.",
                    row.color
                )));
            }
        };
        add(&mut by_color[side], &row.result);
        if let Some(speed) = &row.speed {
            add(speeds.slot(speed, GameRecord::default), &row.result);
        }
        if let Some(opening) = row.opening.as_deref().filter(|o| !o.is_empty()) {
            // Group variations under their opening family: "Sicilian Defense: Najdorf" → "Sicilian Defense".
            let family = js::trim(opening.split(':').next().unwrap_or_default());
            let entry = openings.slot(family, || (GameRecord::default(), 0));
            add(&mut entry.0, &row.result);
            if row.color == "white" {
                entry.1 += 1;
            }
        }
        if (0..7).contains(&row.weekday) {
            add(&mut weekdays[row.weekday as usize], &row.result);
        }
        if (0..24).contains(&row.hour) {
            add(&mut hours[row.hour as usize], &row.result);
        }
        if let (Some(opponent), Some(player)) = (row.opponent_rating, row.player_rating)
            && opponent != 0.0
            && player != 0.0
        {
            let diff = opponent - player;
            if let Some(band) = RATING_BANDS
                .iter()
                .position(|(_, low, high)| diff >= *low && diff <= *high)
            {
                add(&mut opponents[band], &row.result);
            }
        }
        if let Some(band) = LENGTH_BANDS
            .iter()
            .position(|(_, low, high)| row.plies as f64 >= *low && row.plies as f64 <= *high)
        {
            add(&mut lengths[band], &row.result);
        }
        add(endings.slot(&row.status, GameRecord::default), &row.result);
        if row.result == run_kind {
            run += 1;
        } else {
            run_kind = row.result.clone();
            run = 1;
        }
        if run_kind == "win" {
            longest_win = longest_win.max(run);
        }
        if run_kind == "loss" {
            longest_loss = longest_loss.max(run);
        }
    }
    let current = match run_kind.as_str() {
        "win" => run,
        "loss" => -run,
        _ => 0,
    };

    let mut by_speed = speeds.into_items();
    by_speed.sort_by_key(|a| std::cmp::Reverse(a.1.total));
    let mut by_opening: Vec<(String, (GameRecord, i64))> = openings
        .into_items()
        .into_iter()
        .filter(|(_, (record, _))| record.total >= 2)
        .collect();
    by_opening.sort_by_key(|a| std::cmp::Reverse((a.1).0.total));
    by_opening.truncate(25);
    let mut by_ending = endings.into_items();
    by_ending.sort_by_key(|a| std::cmp::Reverse(a.1.total));

    let band = |(label, _, _): &(&str, f64, f64), record: &GameRecord| BandRecord {
        label: (*label).to_string(),
        record: *record,
    };
    Ok(InsightsReport {
        account: account.to_string(),
        total: rows.len() as i64,
        record,
        by_color: ByColor {
            white: by_color[0],
            black: by_color[1],
        },
        by_speed: by_speed
            .into_iter()
            .map(|(speed, record)| SpeedRecord { speed, record })
            .collect(),
        by_opening: by_opening
            .into_iter()
            .map(|(name, (record, as_white))| OpeningRecord {
                name,
                record,
                as_white,
            })
            .collect(),
        by_weekday: weekdays,
        by_hour: hours,
        by_opponent: RATING_BANDS
            .iter()
            .zip(&opponents)
            .map(|(b, r)| band(b, r))
            .collect(),
        by_length: LENGTH_BANDS
            .iter()
            .zip(&lengths)
            .map(|(b, r)| band(b, r))
            .collect(),
        endings: by_ending
            .into_iter()
            .map(|(status, record)| EndingRecord { status, record })
            .collect(),
        streaks: Streaks {
            longest_win,
            longest_loss,
            current,
        },
        accuracy: None,
        time_use: None,
        endgames: None,
        structures: None,
        variants: Vec::new(),
    })
}

/// What replaying the games `query` finds (the newest `REPLAY_LIMIT`) shows: how their
/// endgames went, by the first position at the player's move that `tables` decide (none
/// decided without tables), and the pawn structures each side reached and kept. Neither for a
/// variant. A game from a set-up position starts from its PGN's `FEN`; a game whose moves do
/// not replay is left out.
pub fn replayed_games(
    db: &Connection,
    query: &InsightsQuery,
    now: f64,
    tables: Option<&Tablebase<Chess>>,
) -> Result<(Option<EndgameReport>, Option<StructureReport>)> {
    if query
        .variant
        .is_some_and(|variant| variant != Variant::Standard)
    {
        return Ok((None, None));
    }
    let (clauses, params) = clauses(query, "", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT color, moves, pgn,
           (CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END)
         FROM games WHERE {where_sql} ORDER BY createdAt DESC LIMIT {REPLAY_LIMIT}"
    );
    let mut statement = db.prepare(&sql).sql()?;
    let mut rows = statement.query(params_from_iter(params.iter())).sql()?;
    let mut replays = Replays::default();
    while let Some(row) = rows.next().sql()? {
        let color: String = row.get(0).sql()?;
        let sans: String = row.get(1).sql()?;
        let pgn: Option<String> = row.get(2).sql()?;
        let result: String = row.get(3).sql()?;
        let you = if color == "white" {
            Color::White
        } else {
            Color::Black
        };
        replays.add(you, &result, replayed(pgn.as_deref(), &sans), tables)?;
    }
    Ok(replays.report(tables.is_some()))
}

/// The games replayed for their endgames and pawn structures, newest first.
#[derive(Default)]
pub(crate) struct Replays {
    games: i64,
    won: Slots<EndgameRecord>,
    drawn: Slots<EndgameRecord>,
    yours: Slots<StructureRecord>,
    theirs: Slots<StructureRecord>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Slots::new()
    }
}

impl Replays {
    /// One game the player had `you` in, with its `result` for them, from its start along its
    /// moves (none when they do not replay, which still counts the game): the structures it
    /// kept, and its endgame as `tables` decide it.
    pub(crate) fn add(
        &mut self,
        you: Color,
        result: &str,
        replayed: Option<(Chess, Vec<Move>)>,
        tables: Option<&Tablebase<Chess>>,
    ) -> Result<()> {
        self.games += 1;
        let Some((start, moves)) = replayed else {
            return Ok(());
        };
        for (structure, owner) in patterns::structures_held(&start, &moves) {
            let slots = if owner == you {
                &mut self.yours
            } else {
                &mut self.theirs
            };
            let structure = GameDbStructure::from(structure);
            let entry = slots.slot(&format!("{structure:?}"), || StructureRecord {
                structure,
                record: GameRecord::default(),
            });
            add(&mut entry.record, result);
        }
        let Some(tables) = tables else {
            return Ok(());
        };
        let scanned = endgames::scan(start, &moves, you, tables)?;
        let Some((decided, material)) = scanned.reached else {
            return Ok(());
        };
        let (slots, goal) = match decided {
            2 => (&mut self.won, "win"),
            0 => (&mut self.drawn, "draw"),
            _ => return Ok(()),
        };
        let entry = slots.slot(&material, || EndgameRecord {
            material: material.clone(),
            record: GameRecord::default(),
            let_go: Vec::new(),
        });
        add(&mut entry.record, result);
        if let Some(slip) = scanned.slip.filter(|slip| slip.goal == goal) {
            entry.let_go.push(slip.fen());
        }
        Ok(())
    }

    /// Whether `REPLAY_LIMIT` games are in.
    pub(crate) fn full(&self) -> bool {
        self.games >= REPLAY_LIMIT
    }

    /// The endgames (decided only with `tables`) and the structures of the games added.
    pub(crate) fn report(self, tables: bool) -> (Option<EndgameReport>, Option<StructureReport>) {
        let endgame_lines = |slots: Slots<EndgameRecord>| {
            let mut lines: Vec<EndgameRecord> = slots
                .into_items()
                .into_iter()
                .map(|(_, line)| line)
                .collect();
            lines.sort_by_key(|line| std::cmp::Reverse(line.record.total));
            lines.truncate(ENDGAME_LINES);
            lines
        };
        let structure_lines = |slots: Slots<StructureRecord>| {
            let mut lines: Vec<StructureRecord> = slots
                .into_items()
                .into_iter()
                .map(|(_, line)| line)
                .collect();
            lines.sort_by_key(|line| std::cmp::Reverse(line.record.total));
            lines
        };
        let endgames = EndgameReport {
            tables,
            games: if tables { self.games } else { 0 },
            won: endgame_lines(self.won),
            drawn: endgame_lines(self.drawn),
        };
        let structures = StructureReport {
            games: self.games,
            yours: structure_lines(self.yours),
            theirs: structure_lines(self.theirs),
        };
        (Some(endgames), Some(structures))
    }
}

/// A synced game's start, its PGN's `FEN` or the usual one, and its moves (`sans`) from there.
fn replayed(pgn: Option<&str>, sans: &str) -> Option<(Chess, Vec<Move>)> {
    let fen = pgn.and_then(|pgn| {
        pgn.lines()
            .map(str::trim)
            .take_while(|line| line.starts_with('['))
            .find_map(|line| line.strip_prefix("[FEN \"")?.strip_suffix("\"]"))
    });
    let mut pos: Chess = match fen {
        Some(fen) => fen
            .parse::<Fen>()
            .ok()?
            .into_position(CastlingMode::Standard)
            .ok()?,
        None => Chess::default(),
    };
    let start = pos.clone();
    let mut moves = Vec::new();
    for san in sans.split_whitespace() {
        let played = San::from_ascii(san.as_bytes()).ok()?.to_move(&pos).ok()?;
        pos.play_unchecked(played);
        moves.push(played);
    }
    Some((start, moves))
}

/// One finished game as the report counts it.
pub(crate) struct Row {
    /// The player's colour: `white` or `black`.
    pub color: String,
    /// Its speed's Lichess key (`blitz`); none when its time control is unknown.
    pub speed: Option<String>,
    /// How it ended, as Lichess's status names it (`mate`, `resign`).
    pub status: String,
    pub opening: Option<String>,
    pub opponent_rating: Option<f64>,
    pub player_rating: Option<f64>,
    /// `win`, `loss` or `draw`, for the player.
    pub result: String,
    /// Sunday 0 to Saturday 6, and the hour, in local time; -1 when unknown.
    pub weekday: i64,
    pub hour: i64,
    pub plies: i64,
}

/// Accuracy and mistake counts of the reviewed games (by Lichess or locally), newest first.
/// None when no game in the filter has a finished review.
fn accuracy_report(
    host: &dyn Host,
    db: &Connection,
    query: &InsightsQuery,
    now: f64,
) -> Result<Option<AccuracyReport>> {
    let (clauses, params) = clauses(query, "g.", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT g.color AS color, r.summary AS summary FROM games g JOIN reviews r ON r.key = (
           SELECT r2.key FROM game_reviews gr JOIN reviews r2 ON r2.key = gr.reviewKey
           WHERE gr.gameId = g.id AND r2.complete = 1 ORDER BY r2.updatedAt DESC LIMIT 1)
         WHERE {where_sql} AND r.complete = 1
         ORDER BY g.createdAt DESC LIMIT {REVIEW_LIMIT}"
    );
    let mut statement = db.prepare(&sql).sql()?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .sql()?;
    let mut reviewed = Vec::new();
    for row in rows {
        reviewed.push(row.sql()?);
    }
    let mut mistakes = Mistakes::default();
    for (color, text) in &reviewed {
        mistakes.reviewed();
        // A summary that is not JSON, or is empty, is left out.
        let summary = match serde_json::from_str::<Value>(text) {
            Ok(Value::Null) => {
                host.log(Level::Debug, "insights", "Review summary is empty");
                continue;
            }
            Ok(summary) => summary,
            Err(cause) => {
                host.log(
                    Level::Debug,
                    "insights",
                    &format!("Review summary has invalid JSON: {cause}"),
                );
                continue;
            }
        };
        let Some(side) = summary.get(color.as_str()).filter(|s| truthy(Some(s))) else {
            continue;
        };
        mistakes.side(
            side.get("accuracy").and_then(Value::as_f64),
            side.get("acpl").and_then(Value::as_f64),
            [
                count_of(side.get("inaccuracy")),
                count_of(side.get("mistake")),
                count_of(side.get("blunder")),
            ],
        );
    }
    Ok(mistakes.report())
}

/// The reviewed games' accuracy and mistakes, summed.
#[derive(Default)]
pub(crate) struct Mistakes {
    games: i64,
    accuracy_sum: f64,
    accuracy_games: u32,
    acpl_sum: f64,
    acpl_games: u32,
    /// Inaccuracies, mistakes and blunders.
    counts: [f64; 3],
}

impl Mistakes {
    /// One more reviewed game, counted whether or not its side's figures can be read.
    pub(crate) fn reviewed(&mut self) {
        self.games += 1;
    }

    /// The player's side of a reviewed game: its accuracy, its average centipawn loss, and its
    /// inaccuracies, mistakes and blunders.
    pub(crate) fn side(&mut self, accuracy: Option<f64>, acpl: Option<f64>, counts: [f64; 3]) {
        if let Some(accuracy) = accuracy {
            self.accuracy_sum += accuracy;
            self.accuracy_games += 1;
        }
        if let Some(acpl) = acpl {
            self.acpl_sum += acpl;
            self.acpl_games += 1;
        }
        for (sum, count) in self.counts.iter_mut().zip(counts) {
            *sum += count;
        }
    }

    /// None without a reviewed game.
    pub(crate) fn report(&self) -> Option<AccuracyReport> {
        if self.games == 0 {
            return None;
        }
        let games = self.games as f64;
        let [inaccuracy, mistake, blunder] = self.counts;
        Some(AccuracyReport {
            games: self.games,
            average: (self.accuracy_games > 0)
                .then(|| self.accuracy_sum / f64::from(self.accuracy_games)),
            acpl: (self.acpl_games > 0).then(|| self.acpl_sum / f64::from(self.acpl_games)),
            per_game: MistakeRates {
                inaccuracy: inaccuracy / games,
                mistake: mistake / games,
                blunder: blunder / games,
            },
        })
    }

    /// Whether `REVIEW_LIMIT` games are in.
    pub(crate) fn full(&self) -> bool {
        self.games >= REVIEW_LIMIT
    }
}

/// Time use across the clocked games.
#[derive(Default)]
pub(crate) struct Clocks {
    games: i64,
    /// Results of games where the player's clock fell under a tenth of its start, and of the rest.
    trouble: GameRecord,
    spare: GameRecord,
    /// The player's moves and seconds in each phase.
    phase_moves: [i64; 3],
    phase_seconds: [f64; 3],
    /// At each of `CLOCK_MOVES`: the games that reached it, and the player's and the opponent's
    /// clocks summed as percentages of the start.
    clock_at: [(i64, f64, f64); 2],
    /// Reviewed games, and their moves played short of time and with time to spare.
    reviewed: i64,
    blunders_short: Blunders,
    blunders_spare: Blunders,
}

impl Clocks {
    /// One game: the player's side, its result, its clock and its review's move labels.
    pub(crate) fn add(
        &mut self,
        white: bool,
        result: &str,
        time: &TimeUse,
        verdicts: Option<&[MoveVerdict]>,
    ) {
        self.games += 1;
        let mine = |ply: usize| time.white(ply) == white;
        let plies = time.clocks.len();
        let short = (0..plies)
            .filter(|&ply| mine(ply))
            .any(|ply| time.clocks[ply].is_some_and(|clock| clock < time.initial / 10.0));
        if short {
            add(&mut self.trouble, result);
        } else {
            add(&mut self.spare, result);
        }
        for ply in (0..plies).filter(|&ply| mine(ply)) {
            if let Some(spent) = time.spent(ply) {
                let phase = time.phase(ply);
                self.phase_moves[phase] += 1;
                self.phase_seconds[phase] += spent;
            }
        }
        // A side's nth move is its nth ply: the first side's at 2(n-1), the other's one later.
        let first_white = time.white(0);
        let nth = |side_white: bool, n: usize| 2 * (n - 1) + usize::from(side_white != first_white);
        for (slot, &n) in self.clock_at.iter_mut().zip(CLOCK_MOVES.iter()) {
            let at = |side_white: bool| time.clocks.get(nth(side_white, n)).copied().flatten();
            if let (Some(own), Some(other)) = (at(white), at(!white)) {
                slot.0 += 1;
                slot.1 += 100.0 * own / time.initial;
                slot.2 += 100.0 * other / time.initial;
            }
        }
        if let Some(verdicts) = verdicts {
            self.reviewed += 1;
            for ply in (0..plies.min(verdicts.len())).filter(|&ply| mine(ply)) {
                let Some(short) = time.short_before(ply) else {
                    continue;
                };
                let bucket = if short {
                    &mut self.blunders_short
                } else {
                    &mut self.blunders_spare
                };
                bucket.moves += 1;
                if verdicts[ply].judgment == Some(Judgment::Blunder) {
                    bucket.blunders += 1;
                }
            }
        }
    }

    pub(crate) fn report(&self) -> TimeUseReport {
        let total: f64 = self.phase_seconds.iter().sum();
        let phases = PHASES
            .iter()
            .enumerate()
            .filter(|&(phase, _)| self.phase_moves[phase] > 0)
            .map(|(phase, name)| PhaseTime {
                phase: *name,
                moves: self.phase_moves[phase],
                seconds: self.phase_seconds[phase] / self.phase_moves[phase] as f64,
                share: (total > 0.0).then(|| 100.0 * self.phase_seconds[phase] / total),
            })
            .collect();
        let clock_at = CLOCK_MOVES
            .iter()
            .zip(self.clock_at.iter())
            .filter(|(_, (games, _, _))| *games > 0)
            .map(|(n, (games, own, other))| ClockAt {
                at_move: *n as i64,
                games: *games,
                player: own / *games as f64,
                opponent: other / *games as f64,
            })
            .collect();
        TimeUseReport {
            games: self.games,
            trouble: self.trouble,
            spare: self.spare,
            phases,
            clock_at,
            blunders: (self.reviewed > 0).then_some(BlundersByTime {
                games: self.reviewed,
                short: self.blunders_short,
                spare: self.blunders_spare,
            }),
        }
    }

    /// None without a clocked game.
    pub(crate) fn finish(&self) -> Option<TimeUseReport> {
        (self.games > 0).then(|| self.report())
    }

    /// Whether `CLOCK_LIMIT` games are in.
    pub(crate) fn full(&self) -> bool {
        self.games >= CLOCK_LIMIT
    }
}

/// How the clock was used in the newest real-time games whose PGN has clocks: results when
/// short of time, time per move by phase, the clock against the opponent's, and blunders short
/// of time and with time to spare where a game was reviewed. None without a clocked game.
fn time_use_report(
    host: &dyn Host,
    db: &Connection,
    query: &InsightsQuery,
    now: f64,
) -> Result<Option<TimeUseReport>> {
    let (clauses, params) = clauses(query, "g.", now)?;
    let where_sql = clauses.join(" AND ");
    let sql = format!(
        "SELECT g.color, g.pgn,
           (CASE WHEN g.winner IS NULL THEN 'draw' WHEN g.winner = g.color THEN 'win' ELSE 'loss' END),
           (SELECT r.data FROM game_reviews gr JOIN reviews r ON r.key = gr.reviewKey
            WHERE gr.gameId = g.id AND r.complete = 1 ORDER BY r.updatedAt DESC LIMIT 1)
         FROM games g
         WHERE {where_sql} AND g.pgn LIKE '%[%clk %' AND g.speed != 'correspondence'
         ORDER BY g.createdAt DESC LIMIT {CLOCK_LIMIT}"
    );
    let mut statement = db.prepare(&sql).sql()?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .sql()?;
    let mut clocks = Clocks::default();
    for row in rows {
        let (color, pgn, result, review) = row.sql()?;
        let Some(time) = time_use(&pgn) else {
            continue;
        };
        // A review that cannot be read or analysed leaves the game's moves unlabelled.
        let analysed = review
            .and_then(|data| serde_json::from_str::<StoredReview>(&data).ok())
            .and_then(|data| match super::reviews::analyse_stored(&data) {
                Ok(analysis) => Some(analysis),
                Err(cause) => {
                    host.log(
                        Level::Debug,
                        "insights",
                        &format!("Review left out of time use: {cause}"),
                    );
                    None
                }
            });
        let verdicts = analysed.as_ref().map(|analysis| analysis.moves.as_slice());
        clocks.add(color == "white", &result, &time, verdicts);
    }
    Ok(clocks.finish())
}

/// A count; absent or null is 0.
fn count_of(value: Option<&Value>) -> f64 {
    match value {
        None | Some(Value::Null) => 0.0,
        other => js::to_number(other),
    }
}
