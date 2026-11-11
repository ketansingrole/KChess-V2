//! Arena and Swiss tournaments: the list the timeline shows,
//! one tournament with its leaderboard, joining and leaving, and creating an arena.
//!
//! Lichess's answers are read into typed shapes (`decode`): a required key must be present and
//! well typed, an optional key may be absent but not null (where null is allowed, it reads as
//! absent), and unknown keys pass. A listing skips entries it cannot read;
//! an arena's extras are kept only when all of them read. Eligibility (`playable`, `problem`)
//! follows the Board API rules in `kchess_domain`.

use serde::Deserialize;
use serde_json::{Number, Value};

use super::accounts::{Lichess, Reply, path_segment};
use super::client::Auth;
use super::decode::{Millis, Text, present, read};
use super::online::js_number;
use crate::error::{CoreError, ErrorKind, Result as CoreResult};
use crate::host::Level;
pub use kchess_contracts::types::{
    NewArena, TournamentClock, TournamentClocks, TournamentCondition, TournamentDetail,
    TournamentExtras, TournamentFeatured, TournamentGame, TournamentList, TournamentMe,
    TournamentPlayer, TournamentPodium, TournamentStanding, TournamentStats, TournamentSummary,
    TournamentTeam, TournamentVerdicts,
};
use kchess_contracts::types::{TournamentStatus, TournamentSystem, UsageKind};
use kchess_domain::color::Color;
use kchess_domain::js;

/* ── Lichess's shapes (see `decode` for the key rules) ─────────────────── */

/// A key string, or `{ key, name? }`.
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
    clock: TournamentClock,
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

impl From<RawVerdicts> for TournamentVerdicts {
    fn from(raw: RawVerdicts) -> TournamentVerdicts {
        TournamentVerdicts {
            accepted: raw.accepted,
            list: raw
                .list
                .into_iter()
                .map(|entry| TournamentCondition {
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
    clock: TournamentClock,
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

/// `{ name, rating?, rank? }`.
#[derive(Deserialize)]
struct RawRankedPlayer {
    name: Text<40>,
    #[serde(default, deserialize_with = "present")]
    rating: Option<Number>,
    #[serde(default, deserialize_with = "present")]
    rank: Option<Number>,
}

impl From<RawRankedPlayer> for TournamentPlayer {
    fn from(raw: RawRankedPlayer) -> TournamentPlayer {
        TournamentPlayer {
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

impl From<RawDuelSide> for TournamentPlayer {
    fn from(raw: RawDuelSide) -> TournamentPlayer {
        TournamentPlayer {
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
    c: Option<TournamentClocks>,
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
    stats: Option<TournamentStats>,
}

/* ── Eligibility and dates ─────────────────────────────────────────────── */

/// `compatibility(clock, variant)`: whether KChess can play the games through the Board API.
fn compatibility(clock: &TournamentClock, variant: &str) -> (bool, Option<String>) {
    if kchess_domain::variant::Variant::from_lichess(Some(variant)).is_none() {
        return (
            false,
            Some(format!("KChess cannot show {variant} games yet.")),
        );
    }
    let minutes = clock.limit.as_f64().unwrap_or(0.0) / 60.0;
    let increment = clock.increment.as_f64().unwrap_or(0.0);
    if !kchess_domain::records::time_control::can_board_seek(minutes, increment) {
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

/// The summary of an entry of the arena list, or None when it is not readable.
fn arena_summary(raw: &Value) -> Option<TournamentSummary> {
    let arena: RawArena = read(raw)?;
    let (variant_key, variant_name) = arena.variant.into_parts();
    let (playable, problem) = compatibility(&arena.clock, &variant_key);
    Some(TournamentSummary {
        id: arena.id.into(),
        system: TournamentSystem::Arena,
        name: arena.full_name.into(),
        status: arena_status(&arena.status),
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
fn arena_status(status: &Number) -> TournamentStatus {
    match status.as_f64() {
        Some(20.0) => TournamentStatus::Started,
        Some(30.0) => TournamentStatus::Finished,
        _ => TournamentStatus::Created,
    }
}

/// The Swiss parts a summary does not keep, for a detail.
struct SwissExtras {
    verdicts: Option<RawVerdicts>,
    next_round: Option<RawNextRound>,
}

/// The summary and the Swiss extras of an entry, or None when the entry is not readable.
fn swiss_read(
    raw: &Value,
    team: Option<TournamentTeam>,
) -> Option<(TournamentSummary, SwissExtras)> {
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
        system: TournamentSystem::Swiss,
        name: name.into(),
        status: match status {
            SwissStatus::Created => TournamentStatus::Created,
            SwissStatus::Started => TournamentStatus::Started,
            SwissStatus::Finished => TournamentStatus::Finished,
        },
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

/// The summary of a Swiss entry, or None when it is not readable.
fn swiss_summary(raw: &Value, team: Option<TournamentTeam>) -> Option<TournamentSummary> {
    swiss_read(raw, team).map(|(summary, _)| summary)
}

/// A leaderboard row, or None when it is not readable.
fn standing_of(raw: &Value) -> Option<TournamentStanding> {
    let row: RawStanding = read(raw)?;
    let (scores, fire) = row.sheet.map_or((None, None), |sheet| {
        (sheet.scores.map(String::from), sheet.fire)
    });
    Some(TournamentStanding {
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

/// The featured game, the games in progress, the podium and the totals of an
/// arena. The schema is checked as one: when any part is unreadable, none of them is kept.
pub fn arena_extras(raw: &Value) -> TournamentExtras {
    let Some(extras) = read::<RawExtras>(raw) else {
        return TournamentExtras::default();
    };
    let featured = extras.featured.map(|f| TournamentFeatured {
        id: f.id.into(),
        // Lichess sends the board and side to move only.
        fen: format!(
            "{} - - 0 1",
            f.fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
        ),
        orientation: match f.orientation {
            Some(Side::Black) => Color::Black,
            Some(Side::White) | None => Color::White,
        },
        last_move: f.last_move.map(String::from),
        white: f.white.into(),
        black: f.black.into(),
        clocks: f.c,
    });
    TournamentExtras {
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
                .map(|entry| TournamentPodium {
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

/// A short reason for the Swiss list, never Lichess's raw response. Only a refused login asks
/// to connect again: a 403 for another reason did, and reconnecting changed nothing.
fn team_problem(cause: &CoreError) -> &'static str {
    match cause.kind {
        Some(ErrorKind::Login) => ". Connect the account again to allow it.",
        Some(ErrorKind::RateLimited) => ". Lichess asked to slow down.",
        _ => ".",
    }
}

/// The first `max` UTF-16 units of `text`, as every Lichess text is cut.
fn first_units(text: &str, max: usize) -> String {
    js::slice_utf16(text, max).to_string()
}

/* ── The calls ─────────────────────────────────────────────────────────── */

/// The tournaments of the timeline (`tournaments`). Arenas are listed for everyone; with an
/// account, its teams' upcoming Swiss events are listed too (signed in).
pub async fn tournaments(lichess: &Lichess, account: &str) -> CoreResult<TournamentList> {
    let raw: Value = lichess
        .client()
        .get(
            "/api/tournament",
            &lichess.client().counted(account, UsageKind::Tournament),
        )
        .read_json()
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
        swiss_of_teams(lichess, account, &mut swiss, &mut problems).await
    };
    if let Err(cause) = teams {
        lichess.host().log(
            Level::Warn,
            "tournaments",
            &format!("Could not load teams: {account} {}", cause.message),
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
    swiss: &mut Vec<TournamentSummary>,
    problems: &mut Vec<String>,
) -> CoreResult<()> {
    // Lichess lists a player's teams only to signed-in apps; a rejected login falls back.
    let path = format!("/api/team/of/{}", path_segment(account));
    let teams: Value = lichess
        .as_owner(account, UsageKind::Tournament, |auth| {
            let path = &path;
            async move { lichess.client().get(path, &auth).read_json().await }
        })
        .await?;
    let teams = teams
        .as_array()
        .ok_or_else(|| CoreError::new("Lichess sent an unreadable team list."))?;
    // Lichess asks for one request at a time; a few teams are plenty.
    for raw_team in teams.iter().take(8) {
        let team = TournamentTeam {
            id: raw_team["id"].as_str().unwrap_or("").to_string(),
            name: raw_team["name"].as_str().unwrap_or("").to_string(),
        };
        let path = format!("/api/team/{}/swiss", path_segment(&team.id));
        let mut lines: Vec<Value> = Vec::new();
        // A malformed line fails the team's read as it arrives.
        let outcome = lichess
            .client()
            .get(
                &path,
                &lichess.client().counted(account, UsageKind::Tournament),
            )
            .query(&[("max", "10")])
            .lines(
                || {},
                |line| {
                    let value = serde_json::from_str::<Value>(line)
                        .map_err(|cause| CoreError::new(cause.to_string()))?;
                    lines.push(value);
                    Ok(())
                },
            )
            .await;
        match outcome {
            Ok(()) => {
                for raw in &lines {
                    if let Some(summary) = swiss_summary(raw, Some(team.clone()))
                        .filter(|summary| summary.status != TournamentStatus::Finished)
                    {
                        swiss.push(summary);
                    }
                }
            }
            Err(cause) => {
                lichess.host().log(
                    Level::Warn,
                    "tournaments",
                    &format!("Could not load Swiss events: {} {}", team.id, cause.message),
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
/// in it, when the account has a login; without one the answer is anonymous.
pub async fn tournament(
    lichess: &Lichess,
    system: TournamentSystem,
    id: &str,
    account: &str,
    page: u32,
) -> CoreResult<TournamentDetail> {
    let auth = lichess.optional_login(account, UsageKind::Tournament).await;
    if system == TournamentSystem::Arena {
        arena_detail(lichess, id, page, &auth).await
    } else {
        swiss_detail(lichess, id, &auth).await
    }
}

async fn arena_detail(
    lichess: &Lichess,
    id: &str,
    page: u32,
    auth: &Auth,
) -> CoreResult<TournamentDetail> {
    let path = format!("/api/tournament/{}", path_segment(id));
    let raw: Value = lichess
        .client()
        .get(&path, auth)
        .query(&[("page", page.to_string())])
        .read_json()
        .await?;
    let object = raw
        .as_object()
        .ok_or_else(|| CoreError::new("Lichess sent an unreadable tournament."))?;
    let summary = arena_detail_summary(id, &raw)
        .ok_or_else(|| CoreError::new("Lichess sent an unreadable tournament clock."))?;
    let standing: Vec<TournamentStanding> = object
        .get("standing")
        .and_then(|standing| standing.get("players"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(standing_of)
        .take(50)
        .collect();
    let me = object
        .get("me")
        .filter(|me| me.is_object())
        .map(|me| TournamentMe {
            rank: num_at(me, "rank"),
            withdraw: me.get("withdraw") == Some(&Value::Bool(true)),
            game_id: me.get("gameId").and_then(Value::as_str).map(str::to_string),
        });
    let verdicts = object.get("verdicts").and_then(|verdicts| {
        let list = verdicts.get("list")?.as_array()?;
        Some(TournamentVerdicts {
            accepted: js::truthy(verdicts.get("accepted")),
            list: list
                .iter()
                .take(10)
                .map(|entry| TournamentCondition {
                    condition: first_units(
                        &js::to_string(Some(entry.get("condition").unwrap_or(&Value::Null))),
                        200,
                    ),
                    verdict: first_units(
                        &js::to_string(Some(entry.get("verdict").unwrap_or(&Value::Null))),
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
            .map(|text| first_units(text, 2000)),
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
    let clock: TournamentClock = read(object.get("clock")?)?;
    let variant_key = match object.get("variant") {
        Some(Value::String(key)) => key.clone(),
        _ => "standard".to_string(),
    };
    let status = if js::truthy(object.get("isFinished")) {
        TournamentStatus::Finished
    } else if js::truthy(object.get("secondsToStart")) {
        TournamentStatus::Created
    } else {
        TournamentStatus::Started
    };
    let (playable, problem) = compatibility(&clock, &variant_key);
    Some(TournamentSummary {
        id: id.to_string(),
        system: TournamentSystem::Arena,
        name: first_units(
            &object
                .get("fullName")
                .filter(|name| !name.is_null())
                .map(|name| js::to_string(Some(name)))
                .unwrap_or_else(|| id.to_string()),
            120,
        ),
        status,
        variant: variant_key.clone(),
        variant_name: variant_key,
        // Rated unless Lichess says it is not.
        rated: object.get("rated") != Some(&Value::Bool(false)),
        clock,
        minutes: num_at(raw, "minutes"),
        round: None,
        nb_rounds: None,
        nb_players: num_at(raw, "nbPlayers").unwrap_or_else(|| Number::from(0)),
        starts_at: object
            .get("startsAt")
            .map(|value| parse_date(&js::to_string(Some(value))))
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

async fn swiss_detail(lichess: &Lichess, id: &str, auth: &Auth) -> CoreResult<TournamentDetail> {
    let path = format!("/api/swiss/{}", path_segment(id));
    let raw: Value = lichess.client().get(&path, auth).read_json().await?;
    let (summary, swiss) = swiss_read(&raw, None)
        .ok_or_else(|| CoreError::new("Lichess sent details KChess could not read."))?;
    let mut standing: Vec<TournamentStanding> = Vec::new();
    let results = format!("/api/swiss/{}/results", path_segment(id));
    lichess
        .client()
        .get(&results, auth)
        .query(&[("nb", "50")])
        .lines(
            || {},
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
        verdicts: swiss.verdicts.map(TournamentVerdicts::from),
        next_round_in: swiss.next_round.and_then(|next| next.in_seconds),
        extras: TournamentExtras::default(),
    })
}

/// Joins a tournament as `account`. An arena is joined with pairing as soon as
/// possible; the password is sent only when given. `NeedsReconnect` when the login is missing or
/// lacks the tournament permission.
pub async fn join_tournament(
    lichess: &Lichess,
    system: TournamentSystem,
    id: &str,
    account: &str,
    password: Option<&str>,
) -> CoreResult<Reply<()>> {
    let path = format!(
        "/api/{}/{}/join",
        if system == TournamentSystem::Arena {
            "tournament"
        } else {
            "swiss"
        },
        path_segment(id)
    );
    let password = password
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let pair_me = (system == TournamentSystem::Arena).then(|| "true".to_string());
    lichess
        .as_account(account, UsageKind::Tournament, |auth| async move {
            let mut form: Vec<(&str, &str)> = Vec::new();
            if let Some(password) = password.as_deref() {
                form.push(("password", password));
            }
            if let Some(pair_me) = pair_me.as_deref() {
                form.push(("pairMeAsap", pair_me));
            }
            lichess
                .client()
                .post(&path, &auth)
                .form(&form)
                .read_json::<Value>()
                .await
                .map(|_| ())
        })
        .await
}

/// Withdraws `account` from a tournament.
pub async fn leave_tournament(
    lichess: &Lichess,
    system: TournamentSystem,
    id: &str,
    account: &str,
) -> CoreResult<Reply<()>> {
    let path = format!(
        "/api/{}/{}/withdraw",
        if system == TournamentSystem::Arena {
            "tournament"
        } else {
            "swiss"
        },
        path_segment(id)
    );
    lichess
        .as_account(account, UsageKind::Tournament, |auth| async move {
            lichess
                .client()
                .post(&path, &auth)
                .form::<&str, &str>(&[])
                .read_json::<Value>()
                .await
                .map(|_| ())
        })
        .await
}

/// The form fields of an arena to create. An unset field is left out,
/// and a start date replaces the wait.
pub fn arena_form(arena: &NewArena) -> Vec<(&'static str, String)> {
    let mut form: Vec<(&'static str, String)> = Vec::new();
    if let Some(name) = arena.name.as_deref().filter(|name| !name.is_empty()) {
        form.push(("name", name.to_string()));
    }
    form.push(("clockTime", js_number(arena.clock_time)));
    form.push(("clockIncrement", js_number(arena.clock_increment)));
    form.push(("minutes", js_number(arena.minutes)));
    // A start date (other than 0) replaces the wait.
    let has_start = arena.start_date.is_some_and(|date| date != 0);
    if let Some(wait) = arena.wait_minutes.filter(|_| !has_start) {
        form.push(("waitMinutes", js_number(wait)));
    }
    if let Some(date) = arena.start_date {
        form.push(("startDate", date.to_string()));
    }
    form.push(("variant", arena.variant.key().to_string()));
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

/// Creates an arena run by `account`. A login made before tournaments were
/// requested must reconnect (`NeedsReconnect`).
pub async fn create_tournament(
    lichess: &Lichess,
    account: &str,
    arena: &NewArena,
) -> CoreResult<Reply<TournamentSummary>> {
    let form = arena_form(arena);
    lichess
        .as_account(account, UsageKind::Tournament, |auth| async move {
            let raw: Value = lichess
                .client()
                .post("/api/tournament", &auth)
                .form(&form)
                .read_json()
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
                    CoreError::new(
                        "Lichess created the arena but sent details KChess could not read.",
                    )
                })
        })
        .await
}
