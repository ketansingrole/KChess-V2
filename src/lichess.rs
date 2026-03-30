use std::{collections::HashSet, fmt, time::Duration};

use reqwest::{
    blocking::Client,
    header::{ACCEPT, USER_AGENT},
};
use serde::Deserialize;

use crate::storage::{self, LichessAccount, LichessGame, LichessSyncCursor, StorageError};

const API_BASE: &str = "https://lichess.org/api/games/user";
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub checked_accounts: usize,
    pub updated_accounts: usize,
    pub new_games: usize,
}

#[derive(Debug)]
pub enum SyncError {
    Storage(StorageError),
    Http(reqwest::Error),
    Json(serde_json::Error),
    InvalidResponse(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(err) => write!(f, "{err}"),
            Self::Http(err) => write!(f, "network error: {err}"),
            Self::Json(err) => write!(f, "invalid lichess response: {err}"),
            Self::InvalidResponse(err) => write!(f, "invalid lichess response: {err}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<StorageError> for SyncError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<reqwest::Error> for SyncError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub fn sync_accounts(accounts: &[LichessAccount]) -> Result<SyncSummary, SyncError> {
    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;

    let mut summary = SyncSummary::default();

    for account in accounts {
        let result = sync_account(&client, account)?;
        summary.checked_accounts += 1;
        summary.updated_accounts += usize::from(result.updated);
        summary.new_games += result.new_games;
    }

    Ok(summary)
}

struct AccountSyncResult {
    updated: bool,
    new_games: usize,
}

fn sync_account(client: &Client, account: &LichessAccount) -> Result<AccountSyncResult, SyncError> {
    let newest_remote = fetch_games_page(client, &account.username, None, None, 1)?;
    let newest_cursor = newest_remote
        .first()
        .map(|game| LichessSyncCursor {
            latest_game_id: Some(game.game_id.clone()),
            latest_game_at: Some(game.played_at),
        })
        .unwrap_or_else(LichessSyncCursor::empty);

    let unchanged = newest_cursor.latest_game_id.as_deref() == account.latest_game_id.as_deref()
        && newest_cursor.latest_game_at == account.latest_game_at;

    if unchanged {
        storage::save_lichess_sync(&account.username, &newest_cursor, &[])?;
        return Ok(AccountSyncResult {
            updated: false,
            new_games: 0,
        });
    }

    let fetched_games = collect_games_to_sync(client, account)?;
    let inserted = storage::save_lichess_sync(&account.username, &newest_cursor, &fetched_games)?;

    Ok(AccountSyncResult {
        updated: inserted > 0 || newest_cursor.latest_game_id != account.latest_game_id,
        new_games: inserted,
    })
}

fn collect_games_to_sync(
    client: &Client,
    account: &LichessAccount,
) -> Result<Vec<LichessGame>, SyncError> {
    let mut collected = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut until = None;
    let page_limit = storage::initial_sync_page_size();
    let max_pages = storage::initial_sync_max_pages();
    let since = account.latest_game_at;

    for _ in 0..max_pages {
        let page = fetch_games_page(client, &account.username, since, until, page_limit)?;
        if page.is_empty() {
            break;
        }

        let page_len = page.len();
        let mut oldest_played_at = None;
        let mut reached_known_boundary = false;

        for game in page {
            oldest_played_at = Some(game.played_at);

            if Some(game.game_id.as_str()) == account.latest_game_id.as_deref() {
                reached_known_boundary = true;
                break;
            }

            if let Some(since_at) = since
                && game.played_at < since_at
            {
                reached_known_boundary = true;
                break;
            }

            if seen_ids.insert(game.game_id.clone()) {
                collected.push(game);
            }
        }

        if reached_known_boundary || page_len < page_limit {
            break;
        }

        let Some(oldest_played_at) = oldest_played_at else {
            break;
        };
        until = Some(oldest_played_at.saturating_sub(1));
    }

    Ok(collected)
}

fn fetch_games_page(
    client: &Client,
    username: &str,
    since: Option<i64>,
    until: Option<i64>,
    max: usize,
) -> Result<Vec<LichessGame>, SyncError> {
    let mut query = vec![
        ("max", max.to_string()),
        ("opening", "true".to_string()),
        ("moves", "true".to_string()),
    ];
    if let Some(since) = since {
        query.push(("since", since.to_string()));
    }
    if let Some(until) = until {
        query.push(("until", until.to_string()));
    }

    let response = client
        .get(format!("{API_BASE}/{username}"))
        .header(ACCEPT, "application/x-ndjson")
        .header(USER_AGENT, "kchess/0.1.0")
        .query(&query)
        .send()?
        .error_for_status()?;

    let body = response.text()?;
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }

    parse_ndjson_games(&body, username)
}

fn parse_ndjson_games(body: &str, username: &str) -> Result<Vec<LichessGame>, SyncError> {
    let mut games = Vec::new();

    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let game: ApiGame = serde_json::from_str(line)?;
        let parsed = game.into_storage_game(username).ok_or_else(|| {
            SyncError::InvalidResponse(format!("account {username} not found in returned game"))
        })?;
        games.push(parsed);
    }

    Ok(games)
}

#[derive(Debug, Deserialize)]
struct ApiGame {
    id: String,
    #[serde(rename = "createdAt")]
    created_at: i64,
    #[serde(default)]
    rated: bool,
    #[serde(default)]
    speed: String,
    #[serde(default)]
    perf: String,
    #[serde(default = "default_variant")]
    variant: String,
    #[serde(default)]
    status: String,
    winner: Option<String>,
    players: ApiPlayers,
    opening: Option<ApiOpening>,
    #[serde(default)]
    moves: String,
}

#[derive(Debug, Deserialize)]
struct ApiPlayers {
    white: ApiPlayer,
    black: ApiPlayer,
}

#[derive(Debug, Deserialize)]
struct ApiPlayer {
    user: Option<ApiUser>,
    name: Option<String>,
    rating: Option<i64>,
    #[serde(rename = "ratingDiff")]
    rating_diff: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiOpening {
    name: String,
}

fn default_variant() -> String {
    "standard".to_string()
}

impl ApiGame {
    fn into_storage_game(self, username: &str) -> Option<LichessGame> {
        let ApiPlayers { white, black } = self.players;
        let white_name = white.display_name();
        let black_name = black.display_name();
        let normalized_username = username.to_ascii_lowercase();

        let (color, player, opponent_name, opponent) =
            if white_name.eq_ignore_ascii_case(&normalized_username) {
                ("white".to_string(), white, black_name, black)
            } else if black_name.eq_ignore_ascii_case(&normalized_username) {
                ("black".to_string(), black, white_name, white)
            } else {
                return None;
            };

        Some(LichessGame {
            game_id: self.id,
            account_username: username.to_string(),
            played_at: self.created_at,
            rated: self.rated,
            speed: self.speed,
            perf: self.perf,
            variant: self.variant,
            status: self.status,
            winner: self.winner,
            color,
            opponent_name,
            opponent_rating: opponent.rating,
            player_rating: player.rating,
            rating_diff: player.rating_diff,
            opening_name: self.opening.map(|opening| opening.name),
            moves: self.moves,
        })
    }
}

impl ApiPlayer {
    fn display_name(&self) -> String {
        self.user
            .as_ref()
            .map(|user| user.name.clone())
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| "Anonymous".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ndjson_games_for_account() {
        let body = r#"{"id":"abc123","createdAt":1710000000000,"rated":true,"speed":"blitz","perf":"blitz","variant":"standard","status":"mate","winner":"white","players":{"white":{"user":{"name":"TestUser"},"rating":1810,"ratingDiff":8},"black":{"user":{"name":"Other"},"rating":1790,"ratingDiff":-8}},"opening":{"name":"Italian Game"},"moves":"e4 e5 Nf3 Nc6"}"#;

        let games = parse_ndjson_games(body, "TestUser").expect("parse games");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id, "abc123");
        assert_eq!(games[0].color, "white");
        assert_eq!(games[0].opponent_name, "Other");
        assert_eq!(games[0].rating_diff, Some(8));
    }
}
