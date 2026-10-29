//! Players beyond one profile: following and blocking, many profiles at once, leaderboards, name
//! search, a player's activity, an account's timeline, uploading a game and bookmarked games.
//! Activity and timeline lines are written here, so every frontend shows the same text.

use serde::Serialize;
use serde_json::Value;

use super::accounts::{
    Lichess, LichessGame, Reply, no_such_user, normalize_game, parse_game, path_segment,
};
use super::client::Failure;
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::records::RATING_DISPLAY_NAMES;

/// `/api/users` takes at most this many names a request.
const USERS_PER_REQUEST: usize = 300;
/// How many players a leaderboard shows (Lichess's most).
const LEADERBOARD_SIZE: usize = 100;
/// Lichess searches names from this many characters.
const SEARCH_MIN: usize = 3;
const TIMELINE_SIZE: usize = 30;
const BOOKMARKS_MAX: usize = 100;

/// `LichessRelation` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LichessRelation {
    pub following: bool,
    pub blocking: bool,
}

/// `LeaderboardEntry` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rating: f64,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// `PlayerSuggestion` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PlayerSuggestion {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// `ActivityItem` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
}

/// `ActivityDay` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ActivityDay {
    pub start: f64,
    pub items: Vec<ActivityItem>,
}

/// `TimelineEntry` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TimelineEntry {
    pub date: f64,
    pub item: ActivityItem,
}

/// `ImportedGame` of the contracts.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ImportedGame {
    pub id: String,
    pub url: String,
}

impl Lichess {
    /// `relation`: whether `account` follows or blocks `username` (the profile as `account` sees
    /// it carries both).
    pub async fn relation(
        &self,
        account: &str,
        username: &str,
    ) -> CoreResult<Reply<LichessRelation>> {
        self.as_account(account, "profile", |token| async move {
            let user: Value = self
                .client()
                .get_json(
                    &format!("/api/user/{}", path_segment(username)),
                    &[],
                    Some(&super::client::authorize(&token)),
                    self.lifetime(),
                )
                .await
                .map_err(|failure| no_such_user(username, failure))?;
            Ok(LichessRelation {
                following: user["following"].as_bool().unwrap_or(false),
                blocking: user["blocking"].as_bool().unwrap_or(false),
            })
        })
        .await
    }

    /// `setRelation`: follow, unfollow, block or unblock `username` as `account`. Needs
    /// `follow:write`; a login made before it was requested answers "reconnect".
    pub async fn set_relation(
        &self,
        account: &str,
        username: &str,
        action: &str,
    ) -> CoreResult<Reply<()>> {
        if !matches!(action, "follow" | "unfollow" | "block" | "unblock") {
            return Err(CoreError::new("Invalid action."));
        }
        self.as_account(account, "profile", |token| async move {
            self.client()
                .post_form::<Value>(
                    &format!("/api/rel/{action}/{}", path_segment(username)),
                    &[],
                    Some(&super::client::authorize(&token)),
                    self.lifetime(),
                )
                .await
                .map(|_| ())
        })
        .await
    }

    /// `profiles`: many players at once (`POST /api/users`), in requests of 300; Lichess leaves
    /// out names it does not know.
    pub async fn profiles(&self, usernames: &[String]) -> CoreResult<Vec<Value>> {
        let mut users = Vec::new();
        for chunk in usernames.chunks(USERS_PER_REQUEST) {
            let page: Vec<Value> = self
                .client()
                .policy()
                .with_usage("", "profile", async {
                    self.client()
                        .post_text("/api/users", chunk.join(","), None, self.lifetime())
                        .await
                })
                .await?;
            users.extend(page);
        }
        Ok(users)
    }

    /// `leaderboard`: the top players of `perf`.
    pub async fn leaderboard(&self, perf: &str) -> CoreResult<Vec<LeaderboardEntry>> {
        if perf == "correspondence" {
            return Err(CoreError::new("Lichess has no correspondence leaderboard."));
        }
        let text = self
            .client()
            .policy()
            .with_usage("", "profile", async {
                self.client()
                    .get_text(
                        &format!("/api/player/top/{LEADERBOARD_SIZE}/{}", path_segment(perf)),
                        &[],
                        "application/vnd.lichess.v3+json",
                        None,
                        self.lifetime(),
                    )
                    .await
            })
            .await?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|_| CoreError::new("Lichess returned an invalid leaderboard."))?;
        Ok(read_leaderboard(&body, perf))
    }

    /// `searchPlayers`: players whose names start with `term`.
    pub async fn search_players(&self, term: &str) -> CoreResult<Vec<PlayerSuggestion>> {
        if term.chars().count() < SEARCH_MIN {
            return Ok(Vec::new());
        }
        let body: Value = self
            .client()
            .policy()
            .with_usage("", "profile", async {
                self.client()
                    .get_json(
                        "/api/player/autocomplete",
                        &[("term", term.to_string()), ("object", "true".to_string())],
                        None,
                        self.lifetime(),
                    )
                    .await
            })
            .await?;
        Ok(read_suggestions(&body))
    }

    /// `activity`: what `username` did lately, by day.
    pub async fn activity(&self, username: &str) -> CoreResult<Vec<ActivityDay>> {
        let body: Value = self
            .client()
            .policy()
            .with_usage("", "profile", async {
                self.client()
                    .get_json(
                        &format!("/api/user/{}/activity", path_segment(username)),
                        &[],
                        None,
                        self.lifetime(),
                    )
                    .await
                    .map_err(|failure| no_such_user(username, failure))
            })
            .await?;
        Ok(read_activity(&body))
    }

    /// `timeline`: what the players `account` follows did lately.
    pub async fn timeline(&self, account: &str) -> CoreResult<Reply<Vec<TimelineEntry>>> {
        self.as_account(account, "profile", |token| async move {
            let body: Value = self
                .client()
                .get_json(
                    "/api/timeline",
                    &[("nb", TIMELINE_SIZE.to_string())],
                    Some(&super::client::authorize(&token)),
                    self.lifetime(),
                )
                .await?;
            Ok(read_timeline(&body))
        })
        .await
    }

    /// `importGame`: upload `pgn` to Lichess as `account`, or anonymously when it is empty or its
    /// login is unavailable.
    pub async fn import_game(&self, account: &str, pgn: &str) -> CoreResult<ImportedGame> {
        let token = if account.is_empty() {
            None
        } else {
            self.token_of(account).await
        };
        let answer: Value = self
            .client()
            .policy()
            .with_usage(account, "profile", async {
                self.client()
                    .post_form(
                        "/api/import",
                        &[("pgn", pgn)],
                        token.as_deref().map(super::client::authorize).as_deref(),
                        self.lifetime(),
                    )
                    .await
            })
            .await
            .map_err(|failure: Failure| match failure.status() {
                Some(400) => CoreError::new("Lichess could not read that game."),
                _ => failure.into(),
            })?;
        match (answer["id"].as_str(), answer["url"].as_str()) {
            (Some(id), Some(url)) if url.starts_with("https://") => Ok(ImportedGame {
                id: id.to_string(),
                url: url.to_string(),
            }),
            _ => Err(CoreError::new(
                "Lichess returned an invalid import response.",
            )),
        }
    }

    /// `bookmarkedGames`: the games `account` bookmarked, newest first, from its side.
    pub async fn bookmarked_games(&self, account: &str) -> CoreResult<Reply<Vec<LichessGame>>> {
        let reply = self
            .as_account(account, "profile", |token| async move {
                let mut lines = Vec::new();
                self.client()
                    .ndjson(
                        reqwest::Method::GET,
                        "/api/games/export/bookmarks",
                        &[
                            ("max", BOOKMARKS_MAX.to_string()),
                            ("moves", "false".to_string()),
                            ("opening", "true".to_string()),
                        ],
                        None,
                        Some(&super::client::authorize(&token)),
                        self.lifetime(),
                        |line| {
                            lines.push(line.to_string());
                            Ok(())
                        },
                    )
                    .await?;
                Ok(lines)
            })
            .await?;
        Ok(match reply {
            Reply::Done(lines) => {
                let mut games = Vec::new();
                for line in lines.iter().take(BOOKMARKS_MAX) {
                    games.push(normalize_game(&parse_game(line)?, account));
                }
                Reply::Done(games)
            }
            Reply::NeedsReconnect => Reply::NeedsReconnect,
        })
    }
}

fn label(perf: &str) -> String {
    RATING_DISPLAY_NAMES
        .iter()
        .find(|(key, _)| *key == perf)
        .map_or_else(|| perf.to_string(), |(_, name)| (*name).to_string())
}

fn count(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

fn text_of(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn item(kind: &str, text: String) -> ActivityItem {
    ActivityItem {
        kind: kind.to_string(),
        text,
        username: None,
        game_id: None,
    }
}

/// `{ win, loss, draw }` as "7 wins, 3 losses, 2 draws", leaving out the noughts.
fn score_text(score: &Value) -> String {
    let parts: Vec<String> = [
        ("win", "win", "wins"),
        ("loss", "loss", "losses"),
        ("draw", "draw", "draws"),
    ]
    .iter()
    .filter_map(|(key, one, many)| {
        let n = count(&score[*key]);
        (n > 0).then(|| plural(n, one, many))
    })
    .collect();
    parts.join(", ")
}

/// `rp: { before, after }` as " · 1650 → 1672".
fn rating_change(rp: &Value) -> String {
    match (rp["before"].as_f64(), rp["after"].as_f64()) {
        (Some(before), Some(after)) => format!(" · {} → {}", before.round(), after.round()),
        _ => String::new(),
    }
}

pub fn read_leaderboard(body: &Value, perf: &str) -> Vec<LeaderboardEntry> {
    body["users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|user| {
            let rating = user["perfs"][perf]["rating"].as_f64()?;
            Some(LeaderboardEntry {
                username: text_of(&user["username"])?,
                title: text_of(&user["title"]),
                rating,
                progress: user["perfs"][perf]["progress"].as_f64().unwrap_or(0.0),
                online: user["online"].as_bool(),
            })
        })
        .collect()
}

pub fn read_suggestions(body: &Value) -> Vec<PlayerSuggestion> {
    body["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|user| {
            Some(PlayerSuggestion {
                username: text_of(&user["name"]).or_else(|| text_of(&user["id"]))?,
                title: text_of(&user["title"]),
                online: user["online"].as_bool(),
            })
        })
        .collect()
}

/// One day of `/api/user/{u}/activity` as lines.
fn activity_items(day: &Value) -> Vec<ActivityItem> {
    let mut items = Vec::new();
    if let Some(games) = day["games"].as_object() {
        for (perf, score) in games {
            let n = count(&score["win"]) + count(&score["loss"]) + count(&score["draw"]);
            if n == 0 {
                continue;
            }
            let name = label(perf);
            items.push(item(
                "games",
                format!(
                    "Played {}: {}{}",
                    plural(n, &format!("{name} game"), &format!("{name} games")),
                    score_text(score),
                    rating_change(&score["rp"]),
                ),
            ));
        }
    }
    let puzzles = &day["puzzles"]["score"];
    if puzzles.is_object() {
        let solved = count(&puzzles["win"]);
        let failed = count(&puzzles["loss"]);
        if solved + failed > 0 {
            let mut text = format!("Solved {}", plural(solved, "puzzle", "puzzles"));
            if failed > 0 {
                text.push_str(&format!(", failed {failed}"));
            }
            text.push_str(&rating_change(&puzzles["rp"]));
            items.push(item("puzzles", text));
        }
    }
    for (key, name) in [
        ("storm", "Puzzle Storm"),
        ("racer", "Puzzle Racer"),
        ("streak", "Puzzle Streak"),
    ] {
        let runs = count(&day[key]["runs"]);
        if runs > 0 {
            items.push(item(
                "puzzles",
                format!(
                    "{name}: {}, best {}",
                    plural(runs, "run", "runs"),
                    count(&day[key]["score"])
                ),
            ));
        }
    }
    for best in day["tournaments"]["best"].as_array().into_iter().flatten() {
        let Some(name) = text_of(&best["tournament"]["name"]) else {
            continue;
        };
        let mut text = format!("Ranked #{} in {name}", count(&best["rank"]));
        if let Some(percent) = best["rankPercent"].as_u64() {
            text.push_str(&format!(" (top {percent}%)"));
        }
        text.push_str(&format!(
            " · {}",
            plural(count(&best["nbGames"]), "game", "games")
        ));
        items.push(item("tournament", text));
    }
    let moves = &day["correspondenceMoves"];
    if count(&moves["nb"]) > 0 {
        let games = moves["games"].as_array().map_or(0, Vec::len) as u64;
        items.push(item(
            "correspondence",
            format!(
                "Played {} in {}",
                plural(count(&moves["nb"]), "move", "moves"),
                plural(games, "correspondence game", "correspondence games")
            ),
        ));
    }
    if let Some(ends) = day["correspondenceEnds"].as_object() {
        for ended in ends.values() {
            let score = score_text(&ended["score"]);
            if !score.is_empty() {
                items.push(item(
                    "correspondence",
                    format!("Finished correspondence games: {score}"),
                ));
            }
        }
    }
    let gained = count(&day["follows"]["in"]["nb"])
        .max(day["follows"]["in"]["ids"].as_array().map_or(0, Vec::len) as u64);
    if gained > 0 {
        items.push(item(
            "follow",
            format!("Gained {}", plural(gained, "follower", "followers")),
        ));
    }
    for id in day["follows"]["out"]["ids"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(name) = text_of(id) {
            items.push(ActivityItem {
                username: Some(name.clone()),
                ..item("follow", format!("Started following {name}"))
            });
        }
    }
    for study in day["studies"].as_array().into_iter().flatten() {
        if let Some(name) = text_of(&study["name"]) {
            items.push(item("study", format!("Created the study {name}")));
        }
    }
    for team in day["teams"].as_array().into_iter().flatten() {
        if let Some(name) = text_of(&team["name"]) {
            items.push(item("team", format!("Joined the team {name}")));
        }
    }
    for topic in day["posts"].as_array().into_iter().flatten() {
        if let Some(name) = text_of(&topic["topicName"]) {
            items.push(item("post", format!("Posted in {name}")));
        }
    }
    for simul in day["simuls"].as_array().into_iter().flatten() {
        if let Some(name) = text_of(&simul["name"]) {
            items.push(item("simul", format!("Took part in the simul {name}")));
        }
    }
    for practice in day["practice"].as_array().into_iter().flatten() {
        if let Some(name) = text_of(&practice["name"]) {
            items.push(item("puzzles", format!("Practised {name}")));
        }
    }
    if let Some(months) = day["patron"]["months"].as_u64() {
        items.push(item(
            "patron",
            format!(
                "Supported Lichess for {}",
                plural(months, "month", "months")
            ),
        ));
    }
    if day["stream"].as_bool() == Some(true) {
        items.push(item("stream", "Streamed on Lichess".to_string()));
    }
    items
}

pub fn read_activity(body: &Value) -> Vec<ActivityDay> {
    body.as_array()
        .into_iter()
        .flatten()
        .filter_map(|day| {
            let items = activity_items(day);
            let start = day["interval"]["start"].as_f64()?;
            (!items.is_empty()).then_some(ActivityDay { start, items })
        })
        .collect()
}

/// One `/api/timeline` entry as a line, with `users` naming the ids it mentions.
fn timeline_item(entry: &Value, users: &Value) -> Option<ActivityItem> {
    let data = &entry["data"];
    let name = |id: &Value| -> Option<String> {
        let id = id.as_str()?;
        Some(users[id]["name"].as_str().unwrap_or(id).to_string())
    };
    let by = |text: String| -> Option<ActivityItem> {
        let who = name(&data["userId"])?;
        Some(ActivityItem {
            username: Some(who.clone()),
            ..item("other", format!("{who} {text}"))
        })
    };
    let with_kind = |kind: &str, found: Option<ActivityItem>| {
        found.map(|found| ActivityItem {
            kind: kind.to_string(),
            ..found
        })
    };
    match entry["type"].as_str()? {
        "follow" => {
            let (from, to) = (name(&data["u1"])?, name(&data["u2"])?);
            Some(ActivityItem {
                username: Some(from.clone()),
                ..item("follow", format!("{from} started following {to}"))
            })
        }
        "game-end" => {
            let opponent = name(&data["opponent"]);
            let verb = match data["win"].as_bool() {
                Some(true) => "Won",
                Some(false) => "Lost",
                None => "Drew",
            };
            let against = opponent
                .as_ref()
                .map_or(String::new(), |who| format!(" against {who}"));
            let perf =
                text_of(&data["perf"]).map_or(String::new(), |perf| format!(" ({})", label(&perf)));
            let full = text_of(&data["fullId"])?;
            Some(ActivityItem {
                kind: "games".to_string(),
                text: format!("{verb} a game{against}{perf}"),
                username: opponent,
                game_id: full.get(..8).map(str::to_string),
            })
        }
        "tour-join" => with_kind(
            "tournament",
            by(format!("joined {}", text_of(&data["tourName"])?)),
        ),
        "simul-create" => with_kind(
            "simul",
            by(format!("is hosting {}", text_of(&data["simulName"])?)),
        ),
        "simul-join" => with_kind(
            "simul",
            by(format!("joined {}", text_of(&data["simulName"])?)),
        ),
        "study-like" => with_kind(
            "study",
            by(format!("liked the study {}", text_of(&data["studyName"])?)),
        ),
        "forum-post" => with_kind(
            "post",
            by(format!("posted in {}", text_of(&data["topicName"])?)),
        ),
        "ublog-post" => with_kind(
            "post",
            by(format!("published {}", text_of(&data["title"])?)),
        ),
        "ublog-post-like" => with_kind("post", by(format!("liked {}", text_of(&data["title"])?))),
        "team-join" => with_kind(
            "team",
            by(format!("joined the team {}", text_of(&data["teamId"])?)),
        ),
        "team-create" => with_kind(
            "team",
            by(format!("created the team {}", text_of(&data["teamId"])?)),
        ),
        "plan-start" => with_kind("patron", by("became a Lichess patron".to_string())),
        "plan-renew" => with_kind(
            "patron",
            by(format!(
                "has supported Lichess for {}",
                plural(count(&data["months"]), "month", "months")
            )),
        ),
        "stream-start" => {
            let who = name(&data["id"]).or_else(|| text_of(&data["name"]))?;
            Some(ActivityItem {
                username: Some(who.clone()),
                ..item("stream", format!("{who} started streaming"))
            })
        }
        _ => None,
    }
}

pub fn read_timeline(body: &Value) -> Vec<TimelineEntry> {
    let users = &body["users"];
    body["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(TimelineEntry {
                date: entry["date"].as_f64()?,
                item: timeline_item(entry, users)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_leaderboard_of_one_category() {
        let body = json!({ "users": [
            { "id": "a", "username": "Alice", "title": "GM", "online": true,
              "perfs": { "blitz": { "rating": 3100, "progress": 12 } } },
            { "id": "b", "username": "Bob", "perfs": { "bullet": { "rating": 3000 } } },
        ] });
        let board = read_leaderboard(&body, "blitz");
        assert_eq!(board.len(), 1);
        assert_eq!(board[0].username, "Alice");
        assert_eq!(board[0].title.as_deref(), Some("GM"));
        assert_eq!(board[0].progress, 12.0);
    }

    #[test]
    fn writes_a_day_of_activity_as_lines() {
        let body = json!([{
            "interval": { "start": 1000, "end": 2000 },
            "games": { "blitz": { "win": 7, "loss": 3, "draw": 0, "rp": { "before": 1650, "after": 1672 } } },
            "puzzles": { "score": { "win": 15, "loss": 3, "draw": 0 } },
            "tournaments": { "nb": 1, "best": [{ "tournament": { "id": "x", "name": "Hourly Blitz Arena" },
                "nbGames": 8, "rank": 3, "rankPercent": 5 }] },
            "follows": { "in": { "ids": ["c", "d"] }, "out": { "ids": ["Erin"] } },
        }, { "interval": { "start": 0, "end": 1000 } }]);
        let days = read_activity(&body);
        assert_eq!(days.len(), 1, "a day with nothing to show is left out");
        let lines: Vec<&str> = days[0]
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        assert_eq!(
            lines,
            [
                "Played 10 Blitz games: 7 wins, 3 losses · 1650 → 1672",
                "Solved 15 puzzles, failed 3",
                "Ranked #3 in Hourly Blitz Arena (top 5%) · 8 games",
                "Gained 2 followers",
                "Started following Erin",
            ]
        );
        assert_eq!(days[0].items[4].username.as_deref(), Some("Erin"));
    }

    #[test]
    fn names_timeline_users_and_skips_unknown_entries() {
        let body = json!({
            "entries": [
                { "type": "follow", "data": { "u1": "alice", "u2": "bob" }, "date": 3 },
                { "type": "game-end", "data": { "fullId": "AbCd1234wxyz", "perf": "rapid", "opponent": "bob", "win": true }, "date": 2 },
                { "type": "something-new", "data": {}, "date": 1 },
            ],
            "users": { "alice": { "id": "alice", "name": "Alice" }, "bob": { "id": "bob", "name": "Bob" } },
        });
        let entries = read_timeline(&body);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].item.text, "Alice started following Bob");
        assert_eq!(entries[1].item.text, "Won a game against Bob (Rapid)");
        assert_eq!(entries[1].item.game_id.as_deref(), Some("AbCd1234"));
    }

    #[test]
    fn writes_every_kind_of_activity() {
        let body = json!([{
            "interval": { "start": 5000 },
            "games": { "chess960": { "win": 1 }, "bullet": { "draw": 1, "rp": { "before": 1500.4 } } },
            "puzzles": { "score": { "win": 1, "loss": 0, "rp": { "before": 1500, "after": 1510 } } },
            "storm": { "runs": 2, "score": 31 },
            "racer": { "runs": 1, "score": 20 },
            "streak": { "runs": 0, "score": 9 },
            "tournaments": { "best": [{ "tournament": {}, "rank": 1 }, { "tournament": { "name": "Swiss" }, "rank": 2, "nbGames": 1 }] },
            "correspondenceMoves": { "nb": 1, "games": [{ "id": "a" }] },
            "correspondenceEnds": { "correspondence": { "score": { "win": 1, "loss": 2 } }, "empty": { "score": {} } },
            "follows": { "in": { "nb": 1 }, "out": { "ids": ["", 3] } },
            "studies": [{ "name": "Openings" }, { "id": "unnamed" }],
            "teams": [{ "name": "Club" }],
            "posts": [{ "topicName": "Hello" }],
            "simuls": [{ "name": "Simul" }],
            "practice": [{ "name": "Checkmates" }],
            "patron": { "months": 1 },
            "stream": true,
        }, { "games": { "blitz": { "win": 1 } } }]);
        let days = read_activity(&body);
        // A day without its start is left out.
        assert_eq!(days.len(), 1);
        let lines: Vec<(&str, &str)> = days[0]
            .items
            .iter()
            .map(|item| (item.kind.as_str(), item.text.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                // In the order Lichess lists them.
                ("games", "Played 1 Chess960 game: 1 win"),
                ("games", "Played 1 Bullet game: 1 draw"),
                ("puzzles", "Solved 1 puzzle · 1500 → 1510"),
                ("puzzles", "Puzzle Storm: 2 runs, best 31"),
                ("puzzles", "Puzzle Racer: 1 run, best 20"),
                ("tournament", "Ranked #2 in Swiss · 1 game"),
                ("correspondence", "Played 1 move in 1 correspondence game"),
                (
                    "correspondence",
                    "Finished correspondence games: 1 win, 2 losses"
                ),
                ("follow", "Gained 1 follower"),
                ("study", "Created the study Openings"),
                ("team", "Joined the team Club"),
                ("post", "Posted in Hello"),
                ("simul", "Took part in the simul Simul"),
                ("puzzles", "Practised Checkmates"),
                ("patron", "Supported Lichess for 1 month"),
                ("stream", "Streamed on Lichess"),
            ]
        );
    }

    #[test]
    fn writes_every_kind_of_timeline_entry() {
        let entry = |kind: &str, data: Value| json!({ "type": kind, "data": data, "date": 1 });
        let body = json!({
            "users": { "alice": { "name": "Alice" } },
            "entries": [
                entry("game-end", json!({ "fullId": "AbCd1234wxyz" })),
                entry("game-end", json!({ "fullId": "AbCd1234wxyz", "opponent": "zed", "win": false, "perf": "unknownPerf" })),
                entry("game-end", json!({ "opponent": "alice" })),
                entry("tour-join", json!({ "userId": "alice", "tourName": "Arena" })),
                entry("simul-create", json!({ "userId": "alice", "simulName": "S1" })),
                entry("simul-join", json!({ "userId": "alice", "simulName": "S2" })),
                entry("study-like", json!({ "userId": "alice", "studyName": "Endgames" })),
                entry("forum-post", json!({ "userId": "alice", "topicName": "Hi" })),
                entry("ublog-post", json!({ "userId": "alice", "title": "Blog" })),
                entry("ublog-post-like", json!({ "userId": "alice", "title": "Blog" })),
                entry("team-join", json!({ "userId": "alice", "teamId": "club" })),
                entry("team-create", json!({ "userId": "alice", "teamId": "club" })),
                entry("plan-start", json!({ "userId": "alice" })),
                entry("plan-renew", json!({ "userId": "alice", "months": 2 })),
                entry("stream-start", json!({ "id": "alice" })),
                entry("stream-start", json!({ "name": "Streamer" })),
                entry("tour-join", json!({ "tourName": "No user" })),
                entry("follow", json!({ "u1": "alice" })),
                json!({ "type": "follow", "data": { "u1": "alice", "u2": "bob" } }),
            ],
        });
        let lines: Vec<(String, String, Option<String>)> = read_timeline(&body)
            .into_iter()
            .map(|entry| (entry.item.kind, entry.item.text, entry.item.username))
            .collect();
        let alice = Some("Alice".to_string());
        let expected: Vec<(&str, &str, Option<String>)> = vec![
            ("games", "Drew a game", None),
            (
                "games",
                "Lost a game against zed (unknownPerf)",
                Some("zed".into()),
            ),
            ("tournament", "Alice joined Arena", alice.clone()),
            ("simul", "Alice is hosting S1", alice.clone()),
            ("simul", "Alice joined S2", alice.clone()),
            ("study", "Alice liked the study Endgames", alice.clone()),
            ("post", "Alice posted in Hi", alice.clone()),
            ("post", "Alice published Blog", alice.clone()),
            ("post", "Alice liked Blog", alice.clone()),
            ("team", "Alice joined the team club", alice.clone()),
            ("team", "Alice created the team club", alice.clone()),
            ("patron", "Alice became a Lichess patron", alice.clone()),
            (
                "patron",
                "Alice has supported Lichess for 2 months",
                alice.clone(),
            ),
            ("stream", "Alice started streaming", alice.clone()),
            (
                "stream",
                "Streamer started streaming",
                Some("Streamer".into()),
            ),
        ];
        let expected: Vec<(String, String, Option<String>)> = expected
            .into_iter()
            .map(|(kind, text, user)| (kind.to_string(), text.to_string(), user))
            .collect();
        assert_eq!(lines, expected);
    }

    #[test]
    fn suggests_players_by_name_or_id() {
        let body = json!({ "result": [
            { "name": "Alice", "title": "IM", "online": true },
            { "id": "bob" },
            { "title": "GM" },
        ] });
        let names: Vec<(String, Option<String>)> = read_suggestions(&body)
            .into_iter()
            .map(|user| (user.username, user.title))
            .collect();
        assert_eq!(
            names,
            [
                ("Alice".to_string(), Some("IM".to_string())),
                ("bob".to_string(), None)
            ]
        );
    }
}
