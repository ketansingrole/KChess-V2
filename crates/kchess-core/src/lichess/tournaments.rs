//! Arena and Swiss tournaments (`crates/kchess-node/js/tournaments.ts`): the list the timeline shows,
//! one tournament with its leaderboard, joining and leaving, and creating an arena.
//!
//! Lichess's answers are read into typed shapes (`decode`): a required key must be present and
//! well typed, an optional key may be absent but not null (where null is allowed, it reads as
//! absent), and unknown keys pass. A listing skips entries it cannot read;
//! an arena's extras are kept only when all of them read. Eligibility (`playable`, `problem`)
//! follows the Board API rules in `kchess_domain`.

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::{Failure, TokenSource, authorize};
use super::decode::{Millis, Text, present, read};
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
    /// Arena: the games being played and, once it has finished, its results.
    #[serde(flatten)]
    pub extras: ArenaExtras,
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

/* ── Lichess's shapes (see `decode` for the key rules) ─────────────────── */

/// `variantField`: a key string, or `{ key, name? }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawVariant {
    Key(Text<40>),
    Object {
        key: Text<40>,
        #[serde(default, deserialize_with = "present")]
        name: Option<Text<40>>,
    },
}

impl RawVariant {
    /// The key and its name.
    fn into_parts(self) -> (String, String) {
        match self {
            RawVariant::Key(key) => (key.0.clone(), key.0),
            RawVariant::Object { key, name } => {
                let name = name.map_or_else(|| key.0.clone(), String::from);
                (key.0, name)
            }
        }
    }
}

#[derive(Deserialize)]
struct RawPerf {
    key: Text<30>,
}

#[derive(Deserialize)]
struct RawSchedule {
    #[serde(default, deserialize_with = "present")]
    freq: Option<Text<20>>,
}

#[derive(Deserialize)]
struct RawRating {
    rating: Number,
}

/// An entry of the arena list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawArena {
    id: Text<12>,
    full_name: Text<120>,
    status: Number,
    variant: RawVariant,
    rated: bool,
    clock: Clock,
    #[serde(default, deserialize_with = "present")]
    minutes: Option<Number>,
    nb_players: Number,
    starts_at: Millis,
    #[serde(default, deserialize_with = "present")]
    finishes_at: Option<Millis>,
    #[serde(default, deserialize_with = "present")]
    perf: Option<RawPerf>,
    #[serde(default)]
    schedule: Option<RawSchedule>,
    #[serde(default)]
    max_rating: Option<RawRating>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SwissStatus {
    Created,
    Started,
    Finished,
}

/// A Swiss start: ISO 8601 text, or milliseconds.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawDate {
    Iso(String),
    Millis(Millis),
}

#[derive(Deserialize)]
struct RawNextRound {
    #[serde(default, deserialize_with = "present", rename = "in")]
    in_seconds: Option<Number>,
}

#[derive(Deserialize)]
struct RawVerdict {
    condition: Text<200>,
    verdict: Text<200>,
}

#[derive(Deserialize)]
struct RawVerdicts {
    accepted: bool,
    list: Vec<RawVerdict>,
}

impl From<RawVerdicts> for Verdicts {
    fn from(raw: RawVerdicts) -> Verdicts {
        Verdicts {
            accepted: raw.accepted,
            list: raw
                .list
                .into_iter()
                .map(|entry| Verdict {
                    condition: entry.condition.into(),
                    verdict: entry.verdict.into(),
                })
                .collect(),
        }
    }
}

/// A Swiss event, in a team's list or on its own.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSwiss {
    id: Text<12>,
    name: Text<120>,
    status: SwissStatus,
    variant: RawVariant,
    rated: bool,
    clock: Clock,
    #[serde(default, deserialize_with = "present")]
    round: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    nb_rounds: Option<Number>,
    nb_players: Number,
    starts_at: RawDate,
    #[serde(default, deserialize_with = "present")]
    next_round: Option<RawNextRound>,
    #[serde(default, deserialize_with = "present")]
    verdicts: Option<RawVerdicts>,
}

#[derive(Deserialize)]
struct RawSheet {
    #[serde(default, deserialize_with = "present")]
    scores: Option<Text<400>>,
    #[serde(default, deserialize_with = "present")]
    fire: Option<bool>,
}

/// A leaderboard row.
#[derive(Deserialize)]
struct RawStanding {
    rank: Number,
    #[serde(default, deserialize_with = "present")]
    name: Option<Text<40>>,
    #[serde(default, deserialize_with = "present")]
    username: Option<Text<40>>,
    #[serde(default)]
    title: Option<Text<8>>,
    #[serde(default, deserialize_with = "present")]
    rating: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    score: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    points: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    sheet: Option<RawSheet>,
}

/// `rankedPlayer`: `{ name, rating?, rank? }`.
#[derive(Deserialize)]
struct RawRankedPlayer {
    name: Text<40>,
    #[serde(default, deserialize_with = "present")]
    rating: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    rank: Option<Number>,
}

impl From<RawRankedPlayer> for RankedPlayer {
    fn from(raw: RawRankedPlayer) -> RankedPlayer {
        RankedPlayer {
            name: raw.name.into(),
            rating: raw.rating,
            rank: raw.rank,
        }
    }
}

/// A side of a duel: `{ n, r?, k? }`.
#[derive(Deserialize)]
struct RawDuelSide {
    n: Text<40>,
    #[serde(default, deserialize_with = "present")]
    r: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    k: Option<Number>,
}

impl From<RawDuelSide> for RankedPlayer {
    fn from(raw: RawDuelSide) -> RankedPlayer {
        RankedPlayer {
            name: raw.n.into(),
            rating: raw.r,
            rank: raw.k,
        }
    }
}

#[derive(Deserialize)]
struct RawDuel {
    id: Text<12>,
    p: (RawDuelSide, RawDuelSide),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Side {
    White,
    Black,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFeatured {
    id: Text<12>,
    fen: Text<100>,
    #[serde(default, deserialize_with = "present")]
    orientation: Option<Side>,
    #[serde(default, deserialize_with = "present")]
    last_move: Option<Text<8>>,
    white: RawRankedPlayer,
    black: RawRankedPlayer,
    #[serde(default, deserialize_with = "present")]
    c: Option<Clocks>,
}

#[derive(Deserialize)]
struct RawPodiumEntry {
    name: Text<40>,
    rank: Number,
    #[serde(default, deserialize_with = "present")]
    rating: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    score: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    performance: Option<Number>,
}

/// The extras of an arena detail, read as one.
#[derive(Deserialize)]
struct RawExtras {
    #[serde(default)]
    featured: Option<RawFeatured>,
    #[serde(default, deserialize_with = "present")]
    duels: Option<Vec<RawDuel>>,
    #[serde(default)]
    podium: Option<Vec<RawPodiumEntry>>,
    #[serde(default)]
    stats: Option<Stats>,
}

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
    kchess_domain::calendar::parse_iso_millis(text).unwrap_or(0)
}

/* ── Readers of the Lichess answers ───────────────────────────────────── */

/// `arenaSummary(raw)` of an entry of the arena list, or None when it is not readable.
fn arena_summary(raw: &Value) -> Option<TournamentSummary> {
    let arena: RawArena = read(raw)?;
    let (variant_key, variant_name) = arena.variant.into_parts();
    let (playable, problem) = compatibility(&arena.clock, &variant_key);
    Some(TournamentSummary {
        id: arena.id.into(),
        system: ARENA.into(),
        name: arena.full_name.into(),
        status: arena_status(&arena.status).into(),
        variant: variant_key,
        variant_name,
        rated: arena.rated,
        clock: arena.clock,
        minutes: arena.minutes,
        round: None,
        nb_rounds: None,
        nb_players: arena.nb_players,
        starts_at: arena.starts_at.0,
        finishes_at: arena.finishes_at.map(|at| at.0),
        team: None,
        playable,
        problem,
        perf: arena.perf.map(|perf| perf.key.into()),
        freq: arena
            .schedule
            .and_then(|schedule| schedule.freq)
            .map(String::from),
        max_rating: arena.max_rating.map(|max| max.rating),
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

/// The Swiss parts a summary does not keep, for a detail.
struct SwissExtras {
    verdicts: Option<RawVerdicts>,
    next_round: Option<RawNextRound>,
}

/// `swissSummary(raw, team)` and the Swiss extras, or None when the entry is not readable.
fn swiss_read(raw: &Value, team: Option<Team>) -> Option<(TournamentSummary, SwissExtras)> {
    let RawSwiss {
        id,
        name,
        status,
        variant,
        rated,
        clock,
        round,
        nb_rounds,
        nb_players,
        starts_at,
        next_round,
        verdicts,
    } = read(raw)?;
    let (variant_key, variant_name) = variant.into_parts();
    let (playable, problem) = compatibility(&clock, &variant_key);
    let summary = TournamentSummary {
        id: id.into(),
        system: SWISS.into(),
        name: name.into(),
        status: match status {
            SwissStatus::Created => "created",
            SwissStatus::Started => "started",
            SwissStatus::Finished => "finished",
        }
        .into(),
        variant: variant_key,
        variant_name,
        rated,
        clock,
        minutes: None,
        round,
        nb_rounds,
        nb_players,
        starts_at: match starts_at {
            RawDate::Iso(date) => parse_date(&date),
            RawDate::Millis(at) => at.0,
        },
        finishes_at: None,
        team,
        playable,
        problem,
        perf: None,
        freq: None,
        max_rating: None,
    };
    Some((
        summary,
        SwissExtras {
            verdicts,
            next_round,
        },
    ))
}

/// `swissSummary(raw, team)`, or None when the entry is not readable.
fn swiss_summary(raw: &Value, team: Option<Team>) -> Option<TournamentSummary> {
    swiss_read(raw, team).map(|(summary, _)| summary)
}

/// `standingOf(raw)`: a leaderboard row, or None when it is not readable.
fn standing_of(raw: &Value) -> Option<Standing> {
    let row: RawStanding = read(raw)?;
    let (scores, fire) = row.sheet.map_or((None, None), |sheet| {
        (sheet.scores.map(String::from), sheet.fire)
    });
    Some(Standing {
        rank: row.rank,
        name: row
            .name
            .or(row.username)
            .map_or_else(|| "?".into(), String::from),
        title: row.title.map(String::from),
        rating: row.rating,
        score: row.score.or(row.points),
        sheet: scores,
        fire,
    })
}

/// `arenaExtras(raw)`: the featured game, the games in progress, the podium and the totals of an
/// arena. The schema is checked as one: when any part is unreadable, none of them is kept.
pub fn arena_extras(raw: &Value) -> ArenaExtras {
    let Some(extras) = read::<RawExtras>(raw) else {
        return ArenaExtras::default();
    };
    let featured = extras.featured.map(|f| Featured {
        id: f.id.into(),
        // Lichess sends the board and side to move only.
        fen: format!(
            "{} - - 0 1",
            f.fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
        ),
        orientation: match f.orientation {
            Some(Side::Black) => "black",
            Some(Side::White) | None => "white",
        }
        .into(),
        last_move: f.last_move.map(String::from),
        white: f.white.into(),
        black: f.black.into(),
        clocks: f.c,
    });
    ArenaExtras {
        featured,
        duels: extras.duels.map(|duels| {
            duels
                .into_iter()
                .take(30)
                .map(|duel| TournamentGame {
                    id: duel.id.into(),
                    white: duel.p.0.into(),
                    black: duel.p.1.into(),
                })
                .collect()
        }),
        podium: extras.podium.map(|entries| {
            entries
                .into_iter()
                .take(3)
                .map(|entry| PodiumEntry {
                    name: entry.name.into(),
                    rank: entry.rank,
                    rating: entry.rating,
                    score: entry.score,
                    performance: entry.performance,
                })
                .collect()
        }),
        stats: extras.stats,
    }
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
    let summary = arena_detail_summary(id, &raw).ok_or_else(|| {
        Failure::Core(CoreError::new(
            "Lichess sent an unreadable tournament clock.",
        ))
    })?;
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
        extras: arena_extras(&raw),
    })
}

/// The summary of one arena as Lichess sends it on its own (`GET` or `POST /api/tournament`):
/// a string variant, an ISO start date and `isFinished`/`secondsToStart` in place of a status.
/// None when its clock is unreadable.
fn arena_detail_summary(id: &str, raw: &Value) -> Option<TournamentSummary> {
    let object = raw.as_object()?;
    let clock: Clock = read(object.get("clock")?)?;
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
    let (playable, problem) = compatibility(&clock, &variant_key);
    Some(TournamentSummary {
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
        minutes: num_at(raw, "minutes"),
        round: None,
        nb_rounds: None,
        nb_players: num_at(raw, "nbPlayers").unwrap_or_else(|| Number::from(0)),
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
    })
}

/// A number under `key` of a JSON object, when it is one.
fn num_at(value: &Value, key: &str) -> Option<Number> {
    value.get(key).and_then(Value::as_number).cloned()
}

async fn swiss_detail(
    lichess: &Lichess,
    id: &str,
    auth: Option<&str>,
    cancel: &CancellationToken,
) -> Res<TournamentDetail> {
    let path = format!("/api/swiss/{}", path_segment(id));
    let raw: Value = lichess.client().get_json(&path, &[], auth, cancel).await?;
    let (summary, swiss) = swiss_read(&raw, None).ok_or_else(|| {
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
        verdicts: swiss.verdicts.map(Verdicts::from),
        next_round_in: swiss.next_round.and_then(|next| next.in_seconds),
        extras: ArenaExtras::default(),
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
            // Lichess answers with the arena on its own (as `GET /api/tournament/{id}` does), not
            // with an entry of the arena list.
            let id = raw
                .get("id")
                .and_then(read::<Text<12>>)
                .map(String::from)
                .unwrap_or_default();
            arena_summary(&raw)
                .or_else(|| {
                    (!id.is_empty())
                        .then(|| arena_detail_summary(&id, &raw))
                        .flatten()
                })
                .ok_or_else(|| {
                    Failure::Core(CoreError::new(
                        "Lichess created the arena but sent details KChess could not read.",
                    ))
                })
        })
        .await
        .map_err(Failure::from)
}
