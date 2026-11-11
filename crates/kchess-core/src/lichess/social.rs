//! Players beyond one profile: following and blocking, many profiles at once, leaderboards, name
//! search, a player's activity, the Following list's timeline, uploading a game and bookmarked
//! games.
//! Activity and timeline lines are written here, so every frontend shows the same text.

use serde_json::Value;

use super::accounts::{
    Lichess, Reply, no_such_user, normalize_game, parse_game, path_segment, profile_key,
};
use super::client::Lane;
use crate::error::{CoreError, ErrorKind, Result as CoreResult};
use kchess_contracts::types::{
    ActivityDay, ActivityItem, ImportedGame, LeaderboardEntry, LichessGame, LichessRelation,
    PerfType, PlayerSuggestion, RatingChange, RelationAction, TimelineEntry, UsageKind,
};
use kchess_domain::records::RATING_DISPLAY_NAMES;

/// `/api/users` takes at most this many names a request.
const USERS_PER_REQUEST: usize = 300;
/// How many players a leaderboard shows (Lichess's most).
const LEADERBOARD_SIZE: usize = 100;
/// Lichess searches names from this many characters.
const SEARCH_MIN: usize = 3;
const TIMELINE_SIZE: usize = 30;
/// How many players of the Following list the timeline reads, at most.
const TIMELINE_PLAYERS: usize = 15;
const BOOKMARKS_MAX: usize = 100;

impl Lichess {
    /// `relation`: whether `account` follows or blocks `username` (the profile as `account` sees
    /// it carries both).
    pub async fn relation(
        &self,
        account: &str,
        username: &str,
    ) -> CoreResult<Reply<LichessRelation>> {
        self.as_account(account, UsageKind::Profile, |auth| async move {
            let user: Value = self
                .client()
                .get(&format!("/api/user/{}", path_segment(username)), &auth)
                .read_json()
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
        action: RelationAction,
    ) -> CoreResult<Reply<()>> {
        let action = action.as_str();
        self.as_account(account, UsageKind::Profile, |auth| async move {
            self.client()
                .post(
                    &format!("/api/rel/{action}/{}", path_segment(username)),
                    &auth,
                )
                .form::<&str, &str>(&[])
                .read_json::<Value>()
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
                .post("/api/users", &self.anonymous(UsageKind::Profile))
                .text(chunk.join(","))
                .read_json()
                .await?;
            users.extend(page);
        }
        Ok(users)
    }

    /// `leaderboard`: the top players of `perf`.
    pub async fn leaderboard(&self, perf: PerfType) -> CoreResult<Vec<LeaderboardEntry>> {
        let perf = perf.as_str();
        if perf == "correspondence" {
            return Err(CoreError::new("Lichess has no correspondence leaderboard."));
        }
        let text = self
            .client()
            .get(
                &format!("/api/player/top/{LEADERBOARD_SIZE}/{}", path_segment(perf)),
                &self.anonymous(UsageKind::Profile),
            )
            .accept("application/vnd.lichess.v3+json")
            .read_text(None)
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
            .get(
                "/api/player/autocomplete",
                &self.anonymous(UsageKind::Profile),
            )
            .query(&[("term", term), ("object", "true")])
            .read_json()
            .await?;
        Ok(read_suggestions(&body))
    }

    /// `activity`: what `username` did lately, by day. Cached for a few minutes like a profile,
    /// so the timeline and the player's page share one read.
    pub async fn activity(&self, username: &str) -> CoreResult<Vec<ActivityDay>> {
        let body = self
            .cached_fetch(&profile_key(username, "activity"), username, || async {
                self.client()
                    .get(
                        &format!("/api/user/{}/activity", path_segment(username)),
                        &self.anonymous(UsageKind::Profile),
                    )
                    .read_json()
                    .await
                    .map_err(|failure| no_such_user(username, failure))
            })
            .await?;
        Ok(read_activity(&body))
    }

    /// `timeline`: what the players on the Following list did lately, newest first. Lichess's
    /// own timeline answers only its mobile app's login, so this reads each player's public
    /// activity. A player whose activity cannot be read is left out; when none can be, the first
    /// failure is the answer.
    pub async fn timeline(&self) -> CoreResult<Vec<TimelineEntry>> {
        let players: Vec<String> = self
            .store()
            .accounts()
            .await?
            .into_iter()
            .filter(|row| !row.connected)
            .map(|row| row.username)
            .take(TIMELINE_PLAYERS)
            .collect();
        let mut entries = Vec::new();
        let mut failure = None;
        let mut answered = players.is_empty();
        // One at a time: Lichess asks for that, and answers a burst with a minute's lockout.
        for name in &players {
            match self.activity(name).await {
                Ok(days) => {
                    answered = true;
                    entries.extend(player_timeline(name, days));
                }
                Err(cause) => {
                    self.host().log(
                        crate::host::Level::Warn,
                        "lichess",
                        &format!("Could not read {name}'s activity: {}", cause.message),
                    );
                    failure.get_or_insert(cause);
                }
            }
        }
        if !answered && let Some(cause) = failure {
            return Err(cause);
        }
        // Newest day first; a day keeps each player's lines together, in the list's order.
        entries.sort_by(|a, b| b.date.total_cmp(&a.date));
        entries.truncate(TIMELINE_SIZE);
        Ok(entries)
    }

    /// `importGame`: upload `pgn` to Lichess as `account`, or anonymously when it is empty or its
    /// login is unavailable.
    pub async fn import_game(&self, account: &str, pgn: &str) -> CoreResult<ImportedGame> {
        let auth = self.optional_login(account, UsageKind::Profile).await;
        let answer: Value = self
            .client()
            .post("/api/import", &auth)
            .form(&[("pgn", pgn)])
            .read_json()
            .await
            .map_err(|failure| match failure.kind {
                Some(ErrorKind::Refused) => CoreError::new("Lichess could not read that game."),
                _ => failure,
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
            .as_account(account, UsageKind::Profile, |auth| async move {
                let mut lines = Vec::new();
                self.client()
                    .get("/api/games/export/bookmarks", &auth)
                    .query(&[
                        ("max", BOOKMARKS_MAX.to_string()),
                        ("moves", "false".to_string()),
                        ("opening", "true".to_string()),
                    ])
                    .lane(Lane::Export)
                    .lines(
                        || {},
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
        rating: None,
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
fn rating_change(rp: &Value) -> Option<RatingChange> {
    Some(RatingChange {
        before: rp["before"].as_f64()?.round(),
        after: rp["after"].as_f64()?.round(),
    })
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
    game_items(day, &mut items);
    puzzle_items(day, &mut items);
    event_items(day, &mut items);
    follow_items(day, &mut items);
    community_items(day, &mut items);
    items
}

/// Games played, by speed.
fn game_items(day: &Value, items: &mut Vec<ActivityItem>) {
    if let Some(games) = day["games"].as_object() {
        for (perf, score) in games {
            let n = count(&score["win"]) + count(&score["loss"]) + count(&score["draw"]);
            if n == 0 {
                continue;
            }
            let name = label(perf);
            items.push(ActivityItem {
                rating: rating_change(&score["rp"]),
                ..item(
                    "games",
                    format!(
                        "Played {}: {}",
                        plural(n, &format!("{name} game"), &format!("{name} games")),
                        score_text(score),
                    ),
                )
            });
        }
    }
}

/// Puzzles solved and puzzle runs.
fn puzzle_items(day: &Value, items: &mut Vec<ActivityItem>) {
    let puzzles = &day["puzzles"]["score"];
    if puzzles.is_object() {
        let solved = count(&puzzles["win"]);
        let failed = count(&puzzles["loss"]);
        if solved + failed > 0 {
            let mut text = if solved == 0 {
                format!("Failed {}", plural(failed, "puzzle", "puzzles"))
            } else {
                format!("Solved {}", plural(solved, "puzzle", "puzzles"))
            };
            if solved > 0 && failed > 0 {
                text.push_str(&format!(", failed {failed}"));
            }
            items.push(ActivityItem {
                rating: rating_change(&puzzles["rp"]),
                ..item("puzzles", text)
            });
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
                    "Played {name}: {}, best {}",
                    plural(runs, "run", "runs"),
                    count(&day[key]["score"])
                ),
            ));
        }
    }
}

/// Tournament results and correspondence play.
fn event_items(day: &Value, items: &mut Vec<ActivityItem>) {
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
}

/// Followers gained and players followed.
fn follow_items(day: &Value, items: &mut Vec<ActivityItem>) {
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
}

/// Studies, teams, forum posts, simuls, practice, patronage and streams.
fn community_items(day: &Value, items: &mut Vec<ActivityItem>) {
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

/// `name`'s activity as timeline entries.
fn player_timeline(name: &str, days: Vec<ActivityDay>) -> Vec<TimelineEntry> {
    days.into_iter()
        .flat_map(|day| {
            day.items.into_iter().map(move |item| TimelineEntry {
                date: day.start,
                player: name.to_string(),
                item,
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
                "Played 10 Blitz games: 7 wins, 3 losses",
                "Solved 15 puzzles, failed 3",
                "Ranked #3 in Hourly Blitz Arena (top 5%) · 8 games",
                "Gained 2 followers",
                "Started following Erin",
            ]
        );
        assert_eq!(days[0].items[4].username.as_deref(), Some("Erin"));
        // The rating is its own field, so it lines up beside the text.
        assert_eq!(
            days[0].items[0].rating,
            Some(RatingChange {
                before: 1650.,
                after: 1672.
            })
        );
        assert_eq!(days[0].items[1].rating, None);
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
                ("puzzles", "Solved 1 puzzle"),
                ("puzzles", "Played Puzzle Storm: 2 runs, best 31"),
                ("puzzles", "Played Puzzle Racer: 1 run, best 20"),
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
    fn a_day_of_only_failed_puzzles_says_so() {
        let body = json!([{ "interval": { "start": 0, "end": 1000 },
                            "puzzles": { "score": { "win": 0, "loss": 1, "draw": 0 } } }]);
        assert_eq!(read_activity(&body)[0].items[0].text, "Failed 1 puzzle");
    }

    #[test]
    fn a_players_activity_reads_as_timeline_lines() {
        let body = json!([{
            "interval": { "start": 2000, "end": 3000 },
            "puzzles": { "score": { "win": 2, "loss": 0, "draw": 0 } },
            "follows": { "out": { "ids": ["Erin"] } },
        }]);
        let entries = player_timeline("Dana", read_activity(&body));
        let lines: Vec<(f64, &str, &str, Option<&str>)> = entries
            .iter()
            .map(|entry| {
                (
                    entry.date,
                    entry.player.as_str(),
                    entry.item.text.as_str(),
                    entry.item.username.as_deref(),
                )
            })
            .collect();
        // The player is the entry's; a name in the line stays the line's own link.
        assert_eq!(
            lines,
            [
                (2000., "Dana", "Solved 2 puzzles", None),
                (2000., "Dana", "Started following Erin", Some("Erin")),
            ]
        );
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
