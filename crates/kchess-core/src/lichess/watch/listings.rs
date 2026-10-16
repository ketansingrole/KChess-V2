//! Broadcast listings and the TV and broadcast requests (moved from `watch.rs` without changes).

use serde_json::Value;

use super::{
    BroadcastRoundRef, BroadcastSummary, BroadcastTourDetail, TV_CHANNELS, TvChannel, WatchPlayer,
};
use crate::error::{CoreError, Result as CoreResult};
use crate::lichess::accounts::{Lichess, path_segment};
use kchess_domain::js;

/* ── Broadcast listings (pure) ── */

/// `broadcastSummaries(entries, section)`: the listing's entries that have a readable tournament;
/// only Lichess's own images are kept. A round's section follows its state.
pub fn broadcast_summaries(entries: &[Value], section: &'static str) -> Vec<BroadcastSummary> {
    entries
        .iter()
        .take(30)
        .filter_map(|entry| {
            let tour = entry.get("tour")?;
            let tour = read_tour(tour)?;
            let round = match entry.get("round") {
                None | Some(Value::Null) => None,
                Some(round) => Some(read_round(round)?),
            };
            let ongoing = round.as_ref().is_some_and(|round| round.ongoing);
            let upcoming = round.as_ref().is_some_and(|round| {
                !ongoing && !round.finished && round.finished_at.is_none_or(|at| at == 0.0)
            });
            let section = if ongoing {
                "active"
            } else if upcoming {
                "upcoming"
            } else {
                section
            };
            Some(BroadcastSummary {
                players: tour.players,
                tour_id: tour.id,
                tour_name: tour.name,
                description: tour.description,
                image: tour
                    .image
                    .filter(|image| image.starts_with("https://image.lichess1.org/")),
                round_id: round.as_ref().map(|round| round.id.clone()),
                round_name: round.as_ref().map(|round| round.name.clone()),
                ongoing: round.as_ref().is_some_and(|round| round.ongoing),
                starts_at: round.as_ref().and_then(|round| round.starts_at),
                section,
            })
        })
        .collect()
}

struct TourRaw {
    id: String,
    name: String,
    description: Option<String>,
    image: Option<String>,
    players: Option<String>,
}

struct RoundRaw {
    id: String,
    name: String,
    ongoing: bool,
    finished: bool,
    finished_at: Option<f64>,
    starts_at: Option<i64>,
}

/// A text field of a tournament or round: absent, or a string of at most `max` UTF-16 units.
fn text_field(raw: &Value, key: &str, max: usize) -> Option<Option<String>> {
    match raw.get(key) {
        None | Some(Value::Null) if key != "id" && key != "name" => Some(None),
        Some(Value::String(text)) if js::utf16_len(text) <= max => Some(Some(text.clone())),
        _ => None,
    }
}

fn read_tour(raw: &Value) -> Option<TourRaw> {
    let id = text_field(raw, "id", 12)??;
    let name = text_field(raw, "name", 200)??;
    let description = text_field(raw, "description", 4000)?;
    let image = text_field(raw, "image", 400)?;
    let players = match raw.get("info") {
        None | Some(Value::Null) => None,
        Some(info) => text_field(info, "players", 500)?,
    };
    Some(TourRaw {
        id,
        name,
        description,
        image,
        players,
    })
}

fn read_round(raw: &Value) -> Option<RoundRaw> {
    let id = text_field(raw, "id", 12)??;
    let name = text_field(raw, "name", 120)??;
    let flag = |key: &str| -> Option<bool> {
        match raw.get(key) {
            None | Some(Value::Null) => Some(false),
            Some(Value::Bool(flag)) => Some(*flag),
            Some(_) => None,
        }
    };
    let number = |key: &str| -> Option<Option<f64>> {
        match raw.get(key) {
            None | Some(Value::Null) => Some(None),
            Some(value) => value.as_f64().map(Some),
        }
    };
    Some(RoundRaw {
        id,
        name,
        ongoing: flag("ongoing")?,
        finished: flag("finished")?,
        finished_at: number("finishedAt")?,
        starts_at: number("startsAt")?.map(|n| n as i64),
    })
}

/* ── Requests ── */

/// `tvChannels()`: the TV channels with who is playing on each right now.
pub async fn tv_channels(lichess: &Lichess) -> CoreResult<Vec<TvChannel>> {
    let cancel = lichess.lifetime().clone();
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess
                .client()
                .get_json("/api/tv/channels", &[], None, &cancel)
                .await
        })
        .await?;
    Ok(TV_CHANNELS
        .iter()
        .map(|(key, label)| {
            let entry = raw.get(*key);
            let user = entry.and_then(|entry| entry.get("user"));
            let player = user
                .and_then(|user| user.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(|name| WatchPlayer {
                    name: name.to_string(),
                    title: user
                        .and_then(|user| user.get("title"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    rating: entry
                        .and_then(|entry| entry.get("rating"))
                        .and_then(Value::as_f64),
                });
            TvChannel {
                key,
                label,
                game_id: entry
                    .and_then(|entry| entry.get("gameId"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                player,
            }
        })
        .collect())
}

/// `broadcasts(query)`: the search results for a query, or the broadcasts Lichess features: live,
/// upcoming, then recently finished.
pub async fn broadcasts(
    lichess: &Lichess,
    query: Option<&str>,
) -> CoreResult<Vec<BroadcastSummary>> {
    let cancel = lichess.lifetime().clone();
    let query = query.map(js::trim).filter(|q| !q.is_empty());
    if let Some(query) = query {
        let raw: Value = lichess
            .client()
            .policy()
            .with_usage("", "watch", async {
                lichess
                    .client()
                    .get_json(
                        "/api/broadcast/search",
                        &[("q", query.to_string()), ("page", "1".to_string())],
                        None,
                        &cancel,
                    )
                    .await
            })
            .await?;
        let entries = raw
            .get("currentPageResults")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(broadcast_summaries(&entries, "past"));
    }
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess
                .client()
                .get_json(
                    "/api/broadcast/top",
                    &[("page", "1".to_string())],
                    None,
                    &cancel,
                )
                .await
        })
        .await?;
    let list = |key: &str| {
        raw.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let past = raw
        .pointer("/past/currentPageResults")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut all = broadcast_summaries(&list("active"), "active");
    all.extend(broadcast_summaries(&list("upcoming"), "upcoming"));
    all.extend(broadcast_summaries(&past, "past"));
    Ok(all)
}

/// `broadcastTour(id)`: a broadcast tournament and its rounds (at most 100).
pub async fn broadcast_tour(lichess: &Lichess, id: &str) -> CoreResult<BroadcastTourDetail> {
    let cancel = lichess.lifetime().clone();
    let path = format!("/api/broadcast/{}", path_segment(id));
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess.client().get_json(&path, &[], None, &cancel).await
        })
        .await?;
    let unreadable = || CoreError::new("Lichess sent an unreadable broadcast.");
    let tour = raw.get("tour").and_then(read_tour).ok_or_else(unreadable)?;
    let rounds = raw
        .get("rounds")
        .and_then(Value::as_array)
        .map(|rounds| {
            rounds
                .iter()
                .take(100)
                .filter_map(read_round)
                .map(|round| BroadcastRoundRef {
                    id: round.id,
                    name: round.name,
                    ongoing: round.ongoing,
                    finished: round.finished,
                    starts_at: round.starts_at,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(BroadcastTourDetail {
        id: tour.id,
        name: tour.name,
        description: tour.description,
        rounds,
        default_round_id: raw
            .get("defaultRoundId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}
