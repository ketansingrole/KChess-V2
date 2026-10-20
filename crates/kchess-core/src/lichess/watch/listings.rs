//! Broadcast listings and the TV and broadcast requests (moved from `watch.rs` without changes).

use serde::Deserialize;
use serde_json::Value;

use super::{
    BroadcastRoundRef, BroadcastSummary, BroadcastTourDetail, TV_CHANNELS, TvChannel, WatchPlayer,
};
use crate::error::{CoreError, Result as CoreResult};
use crate::lichess::accounts::{Lichess, path_segment};
use crate::lichess::decode::{Loose, Millis, Text, read};
use kchess_domain::js;

/* ── Broadcast listings (pure) ── */

/// `broadcastSummaries(entries, section)`: the listing's entries that have a readable tournament;
/// only Lichess's own images are kept. A round's section follows its state.
pub fn broadcast_summaries(entries: &[Value], section: &'static str) -> Vec<BroadcastSummary> {
    entries
        .iter()
        .take(30)
        .filter_map(|entry| {
            let ListingRaw { tour, round } = read(entry)?;
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
                players: tour.info.and_then(|info| info.players).map(String::from),
                tour_id: tour.id.into(),
                tour_name: tour.name.into(),
                description: tour.description.map(String::from),
                image: tour
                    .image
                    .map(String::from)
                    .filter(|image| image.starts_with("https://image.lichess1.org/")),
                round_id: round.as_ref().map(|round| round.id.0.clone()),
                round_name: round.as_ref().map(|round| round.name.0.clone()),
                ongoing: round.as_ref().is_some_and(|round| round.ongoing),
                starts_at: round
                    .as_ref()
                    .and_then(|round| round.starts_at)
                    .map(|at| at.0),
                section,
            })
        })
        .collect()
}

/// An entry of a listing: a tournament and, unless null, its current round.
#[derive(Deserialize)]
struct ListingRaw {
    tour: TourRaw,
    #[serde(default)]
    round: Option<RoundRaw>,
}

#[derive(Deserialize)]
struct InfoRaw {
    #[serde(default)]
    players: Option<Text<500>>,
}

/// A broadcast tournament: absent or null optional fields are fine, other shapes are not.
#[derive(Deserialize)]
struct TourRaw {
    id: Text<12>,
    name: Text<200>,
    #[serde(default)]
    description: Option<Text<4000>>,
    #[serde(default)]
    image: Option<Text<400>>,
    #[serde(default)]
    info: Option<InfoRaw>,
}

/// A broadcast round; absent or null flags are false.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoundRaw {
    id: Text<12>,
    name: Text<120>,
    #[serde(default, deserialize_with = "flag")]
    ongoing: bool,
    #[serde(default, deserialize_with = "flag")]
    finished: bool,
    #[serde(default)]
    finished_at: Option<f64>,
    #[serde(default)]
    starts_at: Option<Millis>,
}

/// A flag that may be null (false).
fn flag<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Option::<bool>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Deserialize)]
struct ChannelUserRaw {
    #[serde(default)]
    name: Loose<String>,
    #[serde(default)]
    title: Loose<String>,
}

/// A TV channel: every field is optional and read when it has the right type.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelRaw {
    #[serde(default)]
    user: Loose<ChannelUserRaw>,
    #[serde(default)]
    rating: Loose<f64>,
    #[serde(default)]
    game_id: Loose<String>,
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
            let entry: Option<ChannelRaw> = raw.get(*key).and_then(read);
            let (user, rating, game_id) = entry.map_or((None, None, None), |entry| {
                (entry.user.0, entry.rating.0, entry.game_id.0)
            });
            let player = user.and_then(|user| {
                let name = user.name.0.filter(|name| !name.is_empty())?;
                Some(WatchPlayer {
                    name,
                    title: user.title.0,
                    rating,
                })
            });
            TvChannel {
                key,
                label,
                game_id,
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
    let tour: TourRaw = raw.get("tour").and_then(read).ok_or_else(unreadable)?;
    let rounds = raw
        .get("rounds")
        .and_then(Value::as_array)
        .map(|rounds| {
            rounds
                .iter()
                .take(100)
                .filter_map(read::<RoundRaw>)
                .map(|round| BroadcastRoundRef {
                    id: round.id.into(),
                    name: round.name.into(),
                    ongoing: round.ongoing,
                    finished: round.finished,
                    starts_at: round.starts_at.map(|at| at.0),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(BroadcastTourDetail {
        id: tour.id.into(),
        name: tour.name.into(),
        description: tour.description.map(String::from),
        rounds,
        default_round_id: raw
            .get("defaultRoundId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}
