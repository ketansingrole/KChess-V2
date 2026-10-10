//! Arena and Swiss tournaments (`core/src/services/tournaments.ts`): the list the timeline shows,
//! one tournament with its leaderboard, joining and leaving, and creating an arena.
//!
//! Lichess's answers are read with the rules of the TypeScript schemas (valibot): a required key
//! must be present and well typed, an optional key may be absent but not null (where the schema
//! says nullable, null is a value), and unknown keys pass. A listing skips entries it cannot read;
//! an arena's extras are kept only when all of them read. Eligibility (`playable`, `problem`)
//! follows the Board API rules in `kchess_domain`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::{Failure, TokenSource, authorize};
use super::online::js_number;
use crate::error::CoreError;
use crate::host::Level;

/// `TournamentSystem`: an arena, or a Swiss event.
pub const ARENA: &str = "arena";
pub const SWISS: &str = "swiss";

/// The result type of the calls here: a Lichess or core failure.
pub type Res<T> = std::result::Result<T, Failure>;

/// A tournament's clock: seconds and seconds per move (`clock`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    pub limit: Number,
    pub increment: Number,
}

/// A Swiss event's team (`team`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
}

/// An arena or Swiss tournament in a list (`TournamentSummary`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TournamentSummary {
    pub id: String,
    /// `arena` or `swiss`.
    pub system: String,
    pub name: String,
    /// `created`, `started` or `finished`.
    pub status: String,
    pub variant: String,
    pub variant_name: String,
    pub rated: bool,
    pub clock: Clock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nb_rounds: Option<Number>,
    pub nb_players: Number,
    pub starts_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finishes_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<Team>,
    pub playable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perf: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rating: Option<Number>,
}

/// The tournaments of the timeline, and notes about what could not be read (`TournamentList`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TournamentList {
    pub arenas: Vec<TournamentSummary>,
    pub swiss: Vec<TournamentSummary>,
    pub problems: Vec<String>,
}

/// One row of a leaderboard (`TournamentStanding`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
    pub rank: Number,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Number>,
    /// Arena score sheet: one digit per game, newest last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire: Option<bool>,
}

/// A player in an arena's featured game or a duel (`TournamentGame` side).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedPlayer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<Number>,
}

/// A game between two tournament players (`TournamentGame`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TournamentGame {
    pub id: String,
    pub white: RankedPlayer,
    pub black: RankedPlayer,
}

/// Seconds left on each clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clocks {
    pub white: Number,
    pub black: Number,
}

/// An arena's top game (`featured`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Featured {
    pub id: String,
    pub fen: String,
    pub orientation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub white: RankedPlayer,
    pub black: RankedPlayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clocks: Option<Clocks>,
}

/// An arena's podium entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodiumEntry {
    pub name: String,
    pub rank: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<Number>,
}

/// An arena's totals once it has finished (`stats`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub games: Number,
    pub white_wins: Number,
    pub black_wins: Number,
    pub draws: Number,
    pub berserks: Number,
    pub average_rating: Number,
}

/// The parts of an arena detail read from the same answer (`arenaExtras`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArenaExtras {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub featured: Option<Featured>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duels: Option<Vec<TournamentGame>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub podium: Option<Vec<PodiumEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
}

/// The account's entry in a tournament (`me`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<Number>,
    pub withdraw: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

/// One entry condition and whether the account meets it (`verdicts.list`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub condition: String,
    pub verdict: String,
}

/// Entry conditions and whether the account meets them (`verdicts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdicts {
    pub accepted: bool,
    pub list: Vec<Verdict>,
}

/// One tournament with its leaderboard (`TournamentDetail`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TournamentDetail {
    #[serde(flatten)]
    pub summary: TournamentSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_start: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_to_finish: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub berserkable: Option<bool>,
    pub standing: Vec<Standing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing_page: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<Me>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdicts: Option<Verdicts>,
    /// Swiss: when the next round starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_round_in: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub featured: Option<Featured>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duels: Option<Vec<TournamentGame>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub podium: Option<Vec<PodiumEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
}

/// What an arena to create asks for (`NewArena`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewArena {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub clock_time: Number,
    pub clock_increment: Number,
    pub minutes: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_minutes: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<i64>,
    pub variant: String,
    pub rated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

type Object = Map<String, Value>;

/* ── Schema helpers (valibot's rules over serde_json values) ──────────── */

/// Characters a JavaScript string counts for `maxLength` (UTF-16 code units).
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `v.string()` within `max` UTF-16 units.
fn text(value: &Value, max: usize) -> Option<&str> {
    value.as_str().filter(|text| units(text) <= max)
}

/// `v.number()`.
fn num(value: &Value) -> Option<Number> {
    value.as_number().cloned()
}

/// A whole number read from a finite JSON number (timestamps, in milliseconds).
fn whole(value: &Value) -> Option<i64> {
    value.as_f64().map(|number| number as i64)
}

/// `v.optional(check)` for one key: `Some(None)` when absent, `Some(Some(_))` when it passes,
/// `None` when it is present and fails (null fails too).
fn optional<'a, T>(
    object: &'a Object,
    key: &str,
    check: impl Fn(&'a Value) -> Option<T>,
) -> Option<Option<T>> {
    match object.get(key) {
        None => Some(None),
        Some(value) => check(value).map(Some),
    }
}

/// `v.nullable(check)`: null is a value of its own.
fn nullable<'a, T>(value: &'a Value, check: impl Fn(&'a Value) -> Option<T>) -> Option<Option<T>> {
    if value.is_null() {
        Some(None)
    } else {
        check(value).map(Some)
    }
}

/// `variantField`: a key string, or `{ key, name? }`. Returns the key and its name.
fn variant_of(value: &Value) -> Option<(String, String)> {
    if let Some(key) = text(value, 40) {
        return Some((key.to_string(), key.to_string()));
    }
    let object = value.as_object()?;
    let key = text(object.get("key")?, 40)?.to_string();
    let name = optional(object, "name", |v| text(v, 40))?
        .map(str::to_string)
        .unwrap_or_else(|| key.clone());
    Some((key, name))
}

/// `clockSchema`: an object with numeric `limit` and `increment`.
fn clock_of(value: &Value) -> Option<Clock> {
    let object = value.as_object()?;
    Some(Clock {
        limit: num(object.get("limit")?)?,
        increment: num(object.get("increment")?)?,
    })
}

/// `standingSchema`'s sheet: `{ scores?, fire? }`.
type Sheet = (Option<String>, Option<bool>);

/* ── Eligibility and dates ─────────────────────────────────────────────── */

/// The Board API rules of `timeControl.ts`, read through the domain.
fn board_rule(method: &str, minutes: f64, increment: f64) -> bool {
    matches!(
        kchess_domain::records::call(method, &[json!(minutes), json!(increment)]),
        Some(Ok(Value::Bool(true)))
    )
}

/// `compatibility(clock, variant)`: whether KChess can play the games through the Board API.
fn compatibility(clock: &Clock, variant: &str) -> (bool, Option<String>) {
    if kchess_domain::online_game::variant_from_lichess(Some(variant)).is_none() {
        return (
            false,
            Some(format!("KChess cannot show {variant} games yet.")),
        );
    }
    let minutes = clock.limit.as_f64().unwrap_or(0.0) / 60.0;
    let increment = clock.increment.as_f64().unwrap_or(0.0);
    if !board_rule("canBoardSeek", minutes, increment) {
        return (
            false,
            Some(
                "Lichess lets third-party apps play only Rapid and Classical tournament games (this is faster)."
                    .into(),
            ),
        );
    }
    (true, None)
}

/// `Date.parse(text) || 0` for the ISO 8601 forms Lichess sends (`YYYY-MM-DD`, with an optional
/// time, fraction and offset). Anything else is 0, as an unreadable date is. A bare date or a
/// time without an offset is read as UTC.
pub fn parse_date(text: &str) -> i64 {
    parse_iso(text).unwrap_or(0)
}

fn parse_iso(text: &str) -> Option<i64> {
    let year: i64 = text.get(0..4)?.parse().ok()?;
    if text.get(4..5)? != "-" || text.get(7..8)? != "-" {
        return None;
    }
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut rest = text.get(10..)?;
    let (mut hour, mut minute, mut second, mut millis) = (0i64, 0i64, 0i64, 0i64);
    if let Some(time) = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' ')) {
        hour = time.get(0..2)?.parse().ok()?;
        minute = time.get(3..5)?.parse().ok()?;
        rest = time.get(5..)?;
        if let Some(seconds) = rest.strip_prefix(':') {
            second = seconds.get(0..2)?.parse().ok()?;
            rest = seconds.get(2..)?;
        }
        if let Some(fraction) = rest.strip_prefix('.') {
            let digits: String = fraction
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                return None;
            }
            let padded = format!("{digits:0<3}");
            millis = padded.get(0..3)?.parse().ok()?;
            rest = fraction.get(digits.len()..)?;
        }
    }
    let offset_minutes = if rest.is_empty() || rest == "Z" {
        0
    } else {
        let sign = rest.chars().next()?;
        let zone = rest.get(1..)?;
        let total =
            zone.get(0..2)?.parse::<i64>().ok()? * 60 + zone.get(3..5)?.parse::<i64>().ok()?;
        match sign {
            '+' => total,
            '-' => -total,
            _ => return None,
        }
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1000 + millis)
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/* ── Readers of the Lichess answers ───────────────────────────────────── */

/// `arenaSummary(raw)` of an entry of the arena list, or None when it is not readable.
fn arena_summary(raw: &Value) -> Option<TournamentSummary> {
    let object = raw.as_object()?;
    let id = text(object.get("id")?, 12)?.to_string();
    let name = text(object.get("fullName")?, 120)?.to_string();
    let status = num(object.get("status")?)?;
    let (variant_key, variant_name) = variant_of(object.get("variant")?)?;
    let rated = object.get("rated")?.as_bool()?;
    let clock = clock_of(object.get("clock")?)?;
    let minutes = optional(object, "minutes", num)?;
    let nb_players = num(object.get("nbPlayers")?)?;
    let starts_at = whole(object.get("startsAt")?)?;
    let finishes_at = optional(object, "finishesAt", whole)?;
    // `perf: optional({ key })`: present must be an object with a key.
    let perf = optional(object, "perf", |v| {
        text(v.as_object()?.get("key")?, 30).map(str::to_string)
    })?;
    // `schedule: optional(nullable({ freq? }))`: null is no schedule.
    let freq = match object.get("schedule") {
        None | Some(Value::Null) => None,
        Some(schedule) => {
            optional(schedule.as_object()?, "freq", |f| text(f, 20))?.map(str::to_string)
        }
    };
    // `maxRating: optional(nullable({ rating }))`.
    let max_rating = optional(object, "maxRating", |v| {
        nullable(v, |m| num(m.as_object()?.get("rating")?))
    })?
    .flatten();
    let (playable, problem) = compatibility(&clock, &variant_key);
    Some(TournamentSummary {
        id,
        system: ARENA.into(),
        name,
        status: arena_status(&status).into(),
        variant: variant_key,
        variant_name,
        rated,
        clock,
        minutes,
        round: None,
        nb_rounds: None,
        nb_players,
        starts_at,
        finishes_at,
        team: None,
        playable,
        problem,
        perf,
        freq,
        max_rating,
    })
}

/// `STATUS` of the arena list: Lichess's status numbers (10 created, 20 started, 30 finished).
fn arena_status(status: &Number) -> &'static str {
    match status.as_f64() {
        Some(20.0) => "started",
        Some(30.0) => "finished",
        _ => "created",
    }
}

/// `swissSummary(raw, team)`, or None when the entry is not readable.
fn swiss_summary(raw: &Value, team: Option<Team>) -> Option<TournamentSummary> {
    let object = raw.as_object()?;
    let id = text(object.get("id")?, 12)?.to_string();
    let name = text(object.get("name")?, 120)?.to_string();
    let status = object
        .get("status")?
        .as_str()
        .filter(|status| ["created", "started", "finished"].contains(status))?;
    let (variant_key, variant_name) = variant_of(object.get("variant")?)?;
    let rated = object.get("rated")?.as_bool()?;
    let clock = clock_of(object.get("clock")?)?;
    let round = optional(object, "round", num)?;
    let nb_rounds = optional(object, "nbRounds", num)?;
    let nb_players = num(object.get("nbPlayers")?)?;
    let starts_at = match object.get("startsAt")? {
        Value::String(date) => parse_date(date),
        other => whole(other)?,
    };
    swiss_checked(object)?;
    let (playable, problem) = compatibility(&clock, &variant_key);
    Some(TournamentSummary {
        id,
        system: SWISS.into(),
        name,
        status: status.to_string(),
        variant: variant_key,
        variant_name,
        rated,
        clock,
        minutes: None,
        round,
        nb_rounds,
        nb_players,
        starts_at,
        finishes_at: None,
        team,
        playable,
        problem,
        perf: None,
        freq: None,
        max_rating: None,
    })
}

/// The optional parts of a Swiss entry that the summary does not keep. A malformed one makes the
/// entry unreadable, as the schema does.
fn swiss_checked(object: &Object) -> Option<()> {
    optional(object, "nextRound", |v| {
        optional(v.as_object()?, "in", num).map(|_| ())
    })?;
    optional(object, "verdicts", verdicts_of)?;
    Some(())
}

/// `verdicts`: `{ accepted, list: [{ condition, verdict }] }` with the schema's lengths.
fn verdicts_of(value: &Value) -> Option<Verdicts> {
    let object = value.as_object()?;
    let accepted = object.get("accepted")?.as_bool()?;
    let list = object
        .get("list")?
        .as_array()?
        .iter()
        .map(|entry| {
            let entry = entry.as_object()?;
            Some(Verdict {
                condition: text(entry.get("condition")?, 200)?.to_string(),
                verdict: text(entry.get("verdict")?, 200)?.to_string(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Verdicts { accepted, list })
}

/// `standingOf(raw)`: a leaderboard row, or None when it is not readable.
fn standing_of(raw: &Value) -> Option<Standing> {
    let object = raw.as_object()?;
    let rank = num(object.get("rank")?)?;
    let name = optional(object, "name", |v| text(v, 40))?;
    let username = optional(object, "username", |v| text(v, 40))?;
    // `title: optional(nullable(text(8)))`.
    let title = optional(object, "title", |v| nullable(v, |t| text(t, 8)))?.flatten();
    let rating = optional(object, "rating", num)?;
    let score = optional(object, "score", num)?;
    let points = optional(object, "points", num)?;
    let sheet: Option<Sheet> = optional(object, "sheet", |v| {
        let sheet = v.as_object()?;
        let scores = optional(sheet, "scores", |s| text(s, 400))?;
        let fire = optional(sheet, "fire", Value::as_bool)?;
        Some((scores.map(str::to_string), fire))
    })?;
    let (scores, fire) = sheet.unwrap_or((None, None));
    Some(Standing {
        rank,
        name: name.or(username).unwrap_or("?").to_string(),
        title: title.map(str::to_string),
        rating,
        score: score.or(points),
        sheet: scores,
        fire,
    })
}

/// `rankedPlayer`: `{ name, rating?, rank? }`.
fn ranked_player(value: &Value) -> Option<RankedPlayer> {
    let object = value.as_object()?;
    Some(RankedPlayer {
        name: text(object.get("name")?, 40)?.to_string(),
        rating: optional(object, "rating", num)?,
        rank: optional(object, "rank", num)?,
    })
}

/// A side of a duel: `{ n, r?, k? }`.
fn duel_side(value: &Value) -> Option<RankedPlayer> {
    let side = value.as_object()?;
    Some(RankedPlayer {
        name: text(side.get("n")?, 40)?.to_string(),
        rating: optional(side, "r", num)?,
        rank: optional(side, "k", num)?,
    })
}

/// `arenaExtras(raw)`: the featured game, the games in progress, the podium and the totals of an
/// arena. The schema is checked as one: when any part is unreadable, none of them is kept.
pub fn arena_extras(raw: &Value) -> ArenaExtras {
    arena_extras_checked(raw).unwrap_or_default()
}

fn arena_extras_checked(raw: &Value) -> Option<ArenaExtras> {
    let object = raw.as_object()?;
    // `featured: optional(nullable(...))`.
    let featured = optional(object, "featured", |v| {
        nullable(v, |f| {
            let f = f.as_object()?;
            let id = text(f.get("id")?, 12)?.to_string();
            let fen = text(f.get("fen")?, 100)?.to_string();
            let orientation = optional(f, "orientation", |o| {
                o.as_str()
                    .filter(|side| *side == "white" || *side == "black")
                    .map(str::to_string)
            })?;
            let last_move = optional(f, "lastMove", |m| text(m, 8).map(str::to_string))?;
            let white = ranked_player(f.get("white")?)?;
            let black = ranked_player(f.get("black")?)?;
            let clocks = optional(f, "c", |c| {
                let c = c.as_object()?;
                Some(Clocks {
                    white: num(c.get("white")?)?,
                    black: num(c.get("black")?)?,
                })
            })?;
            Some(Featured {
                id,
                // Lichess sends the board and side to move only.
                fen: format!(
                    "{} - - 0 1",
                    fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
                ),
                orientation: orientation.unwrap_or_else(|| "white".to_string()),
                last_move,
                white,
                black,
                clocks,
            })
        })
    })?
    .flatten();
    // `duels: optional(array)`.
    let duels = optional(object, "duels", |v| {
        v.as_array()?
            .iter()
            .map(|duel| {
                let duel = duel.as_object()?;
                let id = text(duel.get("id")?, 12)?.to_string();
                let pair = duel.get("p")?.as_array()?;
                let [first, second] = pair.as_slice() else {
                    return None;
                };
                Some(TournamentGame {
                    id,
                    white: duel_side(first)?,
                    black: duel_side(second)?,
                })
            })
            .collect::<Option<Vec<_>>>()
    })?;
    // `podium: optional(nullable(array))`.
    let podium = optional(object, "podium", |v| {
        nullable(v, |list| {
            list.as_array()?
                .iter()
                .map(|entry| {
                    let entry = entry.as_object()?;
                    Some(PodiumEntry {
                        name: text(entry.get("name")?, 40)?.to_string(),
                        rank: num(entry.get("rank")?)?,
                        rating: optional(entry, "rating", num)?,
                        score: optional(entry, "score", num)?,
                        performance: optional(entry, "performance", num)?,
                    })
                })
                .collect::<Option<Vec<_>>>()
        })
    })?
    .flatten();
    // `stats: optional(nullable(object))`.
    let stats = optional(object, "stats", |v| {
        nullable(v, |s| {
            let s = s.as_object()?;
            Some(Stats {
                games: num(s.get("games")?)?,
                white_wins: num(s.get("whiteWins")?)?,
                black_wins: num(s.get("blackWins")?)?,
                draws: num(s.get("draws")?)?,
                berserks: num(s.get("berserks")?)?,
                average_rating: num(s.get("averageRating")?)?,
            })
        })
    })?
    .flatten();
    Some(ArenaExtras {
        featured,
        duels: duels.map(|games| games.into_iter().take(30).collect()),
        podium: podium.map(|entries| entries.into_iter().take(3).collect()),
        stats,
    })
}

/// `teamProblem(cause)`: a short reason for the Swiss list, never Lichess's raw response.
fn team_problem(cause: &Failure) -> &'static str {
    match cause.status() {
        Some(401) | Some(403) => ". Connect the account again to allow it.",
        Some(429) => ". Lichess asked to slow down.",
        _ => ".",
    }
}

/// JavaScript truthiness of a JSON value (`!!value`).
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// `String(value)` for a JSON value.
fn js_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".into(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

fn first_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/* ── The calls ─────────────────────────────────────────────────────────── */

/// The tournaments of the timeline (`tournaments`). Arenas are listed for everyone; with an
/// account, its teams' upcoming Swiss events are listed too (signed in).
pub async fn tournaments(lichess: &Lichess, account: &str) -> Res<TournamentList> {
    let cancel = lichess.lifetime().clone();
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage(account, "tournament", async {
            lichess
                .client()
                .get_json::<Value>("/api/tournament", &[], None, &cancel)
                .await
        })
        .await?;
    let mut arenas: Vec<TournamentSummary> = Vec::new();
    // Recently finished arenas stay on the timeline, faded, for context.
    for key in ["started", "created", "finished"] {
        for entry in raw.get(key).and_then(Value::as_array).into_iter().flatten() {
            if let Some(summary) = arena_summary(entry) {
                arenas.push(summary);
            }
        }
    }
    let mut swiss: Vec<TournamentSummary> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let teams = if account.is_empty() {
        Ok(())
    } else {
        swiss_of_teams(lichess, account, &cancel, &mut swiss, &mut problems).await
    };
    if let Err(cause) = teams {
        lichess.host().log(
            Level::Warn,
            "tournaments",
            &format!("Could not load teams: {account} {}", cause.message()),
        );
        problems.push(format!(
            "Couldn’t load the teams of @{account}{}",
            team_problem(&cause)
        ));
    }
    arenas.sort_by_key(|tournament| tournament.starts_at);
    swiss.sort_by_key(|tournament| tournament.starts_at);
    Ok(TournamentList {
        arenas,
        swiss,
        problems,
    })
}

/// The upcoming Swiss events of the account's teams (the first eight teams). A team whose events
/// cannot be read adds a problem and the others still load.
async fn swiss_of_teams(
    lichess: &Lichess,
    account: &str,
    cancel: &CancellationToken,
    swiss: &mut Vec<TournamentSummary>,
    problems: &mut Vec<String>,
) -> Res<()> {
    // Lichess lists a player's teams only to signed-in apps; a rejected login falls back.
    let path = format!("/api/team/of/{}", path_segment(account));
    let teams: Value = lichess
        .client()
        .policy()
        .with_usage(account, "tournament", async {
            lichess
                .as_owner(account, |auth| {
                    let path = path.clone();
                    async move {
                        lichess
                            .client()
                            .get_json::<Value>(&path, &[], auth.as_deref(), cancel)
                            .await
                    }
                })
                .await
        })
        .await?;
    let teams = teams
        .as_array()
        .ok_or_else(|| Failure::Core(CoreError::new("Lichess sent an unreadable team list.")))?;
    // Lichess asks for one request at a time; a few teams are plenty.
    for raw_team in teams.iter().take(8) {
        let team = Team {
            id: raw_team["id"].as_str().unwrap_or("").to_string(),
            name: raw_team["name"].as_str().unwrap_or("").to_string(),
        };
        let path = format!("/api/team/{}/swiss", path_segment(&team.id));
        let mut lines: Vec<Value> = Vec::new();
        // A malformed line fails the team's read as it arrives (`JSON.parse` in `readLines`).
        let outcome = lichess
            .client()
            .policy()
            .with_usage(account, "tournament", async {
                lichess
                    .client()
                    .ndjson(
                        reqwest::Method::GET,
                        &path,
                        &[("max", "10".to_string())],
                        None,
                        None,
                        cancel,
                        |line| {
                            let value = serde_json::from_str::<Value>(line)
                                .map_err(|cause| CoreError::new(cause.to_string()))?;
                            lines.push(value);
                            Ok(())
                        },
                    )
                    .await
            })
            .await;
        match outcome {
            Ok(()) => {
                for raw in &lines {
                    if let Some(summary) = swiss_summary(raw, Some(team.clone()))
                        .filter(|summary| summary.status != "finished")
                    {
                        swiss.push(summary);
                    }
                }
            }
            Err(cause) => {
                lichess.host().log(
                    Level::Warn,
                    "tournaments",
                    &format!(
                        "Could not load Swiss events: {} {}",
                        team.id,
                        cause.message()
                    ),
                );
                problems.push(format!(
                    "Couldn’t load Swiss events of {}{}",
                    team.name,
                    team_problem(&cause)
                ));
            }
        }
    }
    Ok(())
}

/// One tournament with its leaderboard (`tournament`). As `account` it also says whether you are
/// in it, when the account has a login (`tokens`); without one the answer is anonymous.
pub async fn tournament(
    lichess: &Lichess,
    tokens: &dyn TokenSource,
    system: &str,
    id: &str,
    account: &str,
    page: u32,
) -> Res<TournamentDetail> {
    let token = if account.is_empty() {
        None
    } else {
        match tokens.token(account).await {
            Ok(token) => token,
            Err(cause) => {
                lichess.host().log(
                    Level::Debug,
                    "tournaments",
                    &format!("Stored login is unavailable: {account} {}", cause.message),
                );
                None
            }
        }
    };
    let auth = token.as_deref().map(authorize);
    let cancel = lichess.lifetime().clone();
    lichess
        .client()
        .policy()
        .with_usage(account, "tournament", async {
            if system == ARENA {
                arena_detail(lichess, id, page, auth.as_deref(), &cancel).await
            } else {
                swiss_detail(lichess, id, auth.as_deref(), &cancel).await
            }
        })
        .await
}

async fn arena_detail(
    lichess: &Lichess,
    id: &str,
    page: u32,
    auth: Option<&str>,
    cancel: &CancellationToken,
) -> Res<TournamentDetail> {
    let path = format!("/api/tournament/{}", path_segment(id));
    let raw: Value = lichess
        .client()
        .get_json(&path, &[("page", page.to_string())], auth, cancel)
        .await?;
    let object = raw
        .as_object()
        .ok_or_else(|| Failure::Core(CoreError::new("Lichess sent an unreadable tournament.")))?;
    let clock = clock_of(object.get("clock").unwrap_or(&Value::Null)).ok_or_else(|| {
        Failure::Core(CoreError::new(
            "Lichess sent an unreadable tournament clock.",
        ))
    })?;
    let variant_key = match object.get("variant") {
        Some(Value::String(key)) => key.clone(),
        _ => "standard".to_string(),
    };
    let status = if truthy(object.get("isFinished")) {
        "finished"
    } else if truthy(object.get("secondsToStart")) {
        "created"
    } else {
        "started"
    };
    let standing: Vec<Standing> = object
        .get("standing")
        .and_then(|standing| standing.get("players"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(standing_of)
        .take(50)
        .collect();
    let me = object.get("me").filter(|me| me.is_object()).map(|me| Me {
        rank: num_at(me, "rank"),
        withdraw: me.get("withdraw") == Some(&Value::Bool(true)),
        game_id: me.get("gameId").and_then(Value::as_str).map(str::to_string),
    });
    let verdicts = object.get("verdicts").and_then(|verdicts| {
        let list = verdicts.get("list")?.as_array()?;
        Some(Verdicts {
            accepted: truthy(verdicts.get("accepted")),
            list: list
                .iter()
                .take(10)
                .map(|entry| Verdict {
                    condition: first_chars(
                        &js_text(entry.get("condition").unwrap_or(&Value::Null)),
                        200,
                    ),
                    verdict: first_chars(
                        &js_text(entry.get("verdict").unwrap_or(&Value::Null)),
                        200,
                    ),
                })
                .collect(),
        })
    });
    let extras = arena_extras(&raw);
    let (playable, problem) = compatibility(&clock, &variant_key);
    let summary = TournamentSummary {
        id: id.to_string(),
        system: ARENA.into(),
        name: first_chars(
            &object
                .get("fullName")
                .filter(|name| !name.is_null())
                .map(js_text)
                .unwrap_or_else(|| id.to_string()),
            120,
        ),
        status: status.into(),
        variant: variant_key.clone(),
        variant_name: variant_key,
        // `raw.rated !== false`.
        rated: object.get("rated") != Some(&Value::Bool(false)),
        clock,
        minutes: num_at(&raw, "minutes"),
        round: None,
        nb_rounds: None,
        nb_players: num_at(&raw, "nbPlayers").unwrap_or_else(|| Number::from(0)),
        starts_at: object
            .get("startsAt")
            .map(|value| parse_date(&js_text(value)))
            .unwrap_or(0),
        finishes_at: None,
        team: None,
        playable,
        problem,
        perf: None,
        freq: None,
        max_rating: None,
    };
    Ok(TournamentDetail {
        summary,
        description: object
            .get("description")
            .and_then(Value::as_str)
            .map(|text| first_chars(text, 2000)),
        seconds_to_start: num_at(&raw, "secondsToStart"),
        seconds_to_finish: num_at(&raw, "secondsToFinish"),
        berserkable: Some(object.get("berserkable") == Some(&Value::Bool(true))),
        standing,
        standing_page: Some(Number::from(page)),
        me,
        verdicts,
        next_round_in: None,
        featured: extras.featured,
        duels: extras.duels,
        podium: extras.podium,
        stats: extras.stats,
    })
}

/// A number under `key` of a JSON object, when it is one.
fn num_at(value: &Value, key: &str) -> Option<Number> {
    value.get(key).and_then(num)
}

async fn swiss_detail(
    lichess: &Lichess,
    id: &str,
    auth: Option<&str>,
    cancel: &CancellationToken,
) -> Res<TournamentDetail> {
    let path = format!("/api/swiss/{}", path_segment(id));
    let raw: Value = lichess.client().get_json(&path, &[], auth, cancel).await?;
    let summary = swiss_summary(&raw, None).ok_or_else(|| {
        Failure::Core(CoreError::new(
            "Lichess sent details KChess could not read.",
        ))
    })?;
    let mut standing: Vec<Standing> = Vec::new();
    let results = format!("/api/swiss/{}/results", path_segment(id));
    lichess
        .client()
        .ndjson(
            reqwest::Method::GET,
            &results,
            &[("nb", "50".to_string())],
            None,
            auth,
            cancel,
            |line| {
                let value = serde_json::from_str::<Value>(line)
                    .map_err(|cause| CoreError::new(cause.to_string()))?;
                if let Some(row) = standing_of(&value) {
                    standing.push(row);
                }
                Ok(())
            },
        )
        .await?;
    Ok(TournamentDetail {
        summary,
        description: None,
        seconds_to_start: None,
        seconds_to_finish: None,
        berserkable: None,
        standing,
        standing_page: None,
        me: None,
        verdicts: raw.get("verdicts").and_then(verdicts_of),
        next_round_in: raw.get("nextRound").and_then(|next| num_at(next, "in")),
        featured: None,
        duels: None,
        podium: None,
        stats: None,
    })
}

/// Joins a tournament as `account` (`joinTournament`). An arena is joined with pairing as soon as
/// possible; the password is sent only when given. `NeedsReconnect` when the login is missing or
/// lacks the tournament permission.
pub async fn join_tournament(
    lichess: &Lichess,
    system: &str,
    id: &str,
    account: &str,
    password: Option<&str>,
) -> Res<Reply<()>> {
    let cancel = lichess.lifetime().clone();
    let path = format!(
        "/api/{}/{}/join",
        if system == ARENA {
            "tournament"
        } else {
            "swiss"
        },
        path_segment(id)
    );
    let password = password
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let pair_me = (system == ARENA).then(|| "true".to_string());
    lichess
        .as_account(account, "tournament", |token| async move {
            let mut form: Vec<(&str, &str)> = Vec::new();
            if let Some(password) = password.as_deref() {
                form.push(("password", password));
            }
            if let Some(pair_me) = pair_me.as_deref() {
                form.push(("pairMeAsap", pair_me));
            }
            lichess
                .client()
                .post_form::<Value>(&path, &form, Some(&authorize(&token)), &cancel)
                .await
                .map(|_| ())
        })
        .await
        .map_err(Failure::from)
}

/// Withdraws `account` from a tournament (`leaveTournament`).
pub async fn leave_tournament(
    lichess: &Lichess,
    system: &str,
    id: &str,
    account: &str,
) -> Res<Reply<()>> {
    let cancel = lichess.lifetime().clone();
    let path = format!(
        "/api/{}/{}/withdraw",
        if system == ARENA {
            "tournament"
        } else {
            "swiss"
        },
        path_segment(id)
    );
    lichess
        .as_account(account, "tournament", |token| async move {
            lichess
                .client()
                .post_form::<Value>(&path, &[], Some(&authorize(&token)), &cancel)
                .await
                .map(|_| ())
        })
        .await
        .map_err(Failure::from)
}

/// The form fields of an arena to create (`createTournament`'s body). An unset field is left out,
/// and a start date replaces the wait.
pub fn arena_form(arena: &NewArena) -> Vec<(&'static str, String)> {
    let mut form: Vec<(&'static str, String)> = Vec::new();
    if let Some(name) = arena.name.as_deref().filter(|name| !name.is_empty()) {
        form.push(("name", name.to_string()));
    }
    form.push((
        "clockTime",
        js_number(arena.clock_time.as_f64().unwrap_or(0.0)),
    ));
    form.push((
        "clockIncrement",
        js_number(arena.clock_increment.as_f64().unwrap_or(0.0)),
    ));
    form.push(("minutes", js_number(arena.minutes.as_f64().unwrap_or(0.0))));
    // `waitMinutes: startDate ? undefined : waitMinutes`: a non-zero start date is truthy.
    let has_start = arena.start_date.is_some_and(|date| date != 0);
    if let Some(wait) = arena.wait_minutes.as_ref().filter(|_| !has_start) {
        form.push(("waitMinutes", js_number(wait.as_f64().unwrap_or(0.0))));
    }
    if let Some(date) = arena.start_date {
        form.push(("startDate", date.to_string()));
    }
    form.push(("variant", arena.variant.clone()));
    form.push(("rated", arena.rated.to_string()));
    if let Some(password) = arena.password.as_deref().filter(|value| !value.is_empty()) {
        form.push(("password", password.to_string()));
    }
    if let Some(description) = arena
        .description
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        form.push(("description", description.to_string()));
    }
    form
}

/// Creates an arena run by `account` (`createTournament`). A login made before tournaments were
/// requested must reconnect (`NeedsReconnect`).
pub async fn create_tournament(
    lichess: &Lichess,
    account: &str,
    arena: &NewArena,
) -> Res<Reply<TournamentSummary>> {
    let cancel = lichess.lifetime().clone();
    let form = arena_form(arena);
    lichess
        .as_account(account, "tournament", |token| async move {
            let pairs: Vec<(&str, &str)> = form
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();
            let raw = lichess
                .client()
                .post_form::<Value>("/api/tournament", &pairs, Some(&authorize(&token)), &cancel)
                .await?;
            arena_summary(&raw).ok_or_else(|| {
                Failure::Core(CoreError::new(
                    "Lichess created the arena but sent details KChess could not read.",
                ))
            })
        })
        .await
        .map_err(Failure::from)
}
