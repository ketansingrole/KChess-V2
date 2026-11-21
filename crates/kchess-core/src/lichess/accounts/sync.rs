//! The game sync: each connected account's games, page by page from where the last sync
//! stopped, the games still waiting on Lichess's analysis looked at again, and the analysis
//! saved with them.

use super::*;

/// Lichess returns at most this many games per page of a sync.
const SYNC_PAGE: usize = 1000;
/// Overlap of the incremental sync window, for clock skew.
const SYNC_OVERLAP_MS: i64 = 60_000;

impl Lichess {
    /// Syncs the connected accounts (or one of them), in turn. Every account still
    /// syncs when one fails; the first failure is reported afterwards.
    pub async fn sync_games(&self, username: Option<&str>) -> CoreResult<()> {
        // Taken before the accounts are read: data cleared after this resets a cursor read here,
        // and that account's sync stops rather than going on from the old one.
        let epochs = lock(&self.account_epochs).clone();
        let rows = self.store.accounts().await?;
        let rows: Vec<&AccountRow> = rows
            .iter()
            .filter(|row| {
                username.is_none_or(|wanted| row.username.to_lowercase() == wanted.to_lowercase())
            })
            .collect();
        let play_times = self.play_times(&rows).await;
        let mut first_error: Option<CoreError> = None;
        for row in rows {
            let key = row.username.to_lowercase();
            let play_time = play_times.get(&key).copied();
            let epoch = epochs.get(&key).copied().unwrap_or(0);
            if let Err(cause) = self.sync_account(row, play_time, epoch).await {
                self.host.log(
                    Level::Warn,
                    "lichess",
                    &format!("Game sync failed: {} {}", row.username, cause.message),
                );
                first_error.get_or_insert(cause);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Each player's Lichess play time, read before their games are: it grows with every
    /// finished game, so a profile with more than the one saved at the sync has games to sync.
    /// One request covers 300 players; without an answer the sync goes ahead and saves none.
    async fn play_times(&self, rows: &[&AccountRow]) -> HashMap<String, i64> {
        let names: Vec<String> = rows.iter().map(|row| row.username.clone()).collect();
        match self.profiles(&names).await {
            Ok(users) => users
                .iter()
                .filter_map(|user| Some((user.username.to_lowercase(), user.played_seconds()?)))
                .collect(),
            Err(cause) => {
                self.host.log(
                    Level::Warn,
                    "lichess",
                    &format!(
                        "Play times before the sync were not read: {}",
                        cause.message
                    ),
                );
                HashMap::new()
            }
        }
    }

    /// One sync per account at a time: a second request joins the running one.
    /// A logout removes the entry without notifying the waiter; the waiter stays until
    /// the in-flight run finishes and sends. A new generation started after cancel may
    /// briefly run concurrently with the old one, but both recheck the account epoch
    /// before every write (`current()`), so stale writes are skipped and only the
    /// current generation advances the cursor.
    async fn sync_account(
        &self,
        row: &AccountRow,
        play_time: Option<i64>,
        epoch: u64,
    ) -> CoreResult<usize> {
        let run = async {
            let result = self.run_sync(row, play_time, epoch).await;
            if let Ok(fetched) = &result {
                self.host.log(
                    Level::Info,
                    "lichess",
                    &format!("Game sync completed: {} fetched={fetched}", row.username),
                );
            }
            result
        };
        self.syncs
            .run(&row.username.to_lowercase(), run, || {
                Err(CoreError::new("Sync was cancelled."))
            })
            .await
    }

    /// `row` as read under the account's `epoch`.
    async fn run_sync(
        &self,
        row: &AccountRow,
        play_time: Option<i64>,
        epoch: u64,
    ) -> CoreResult<usize> {
        let name = row.username.as_str();
        let current = || self.account_epoch(name) == epoch;
        let started_at = now_ms();
        async {
            self.refresh_pending_games(name, &current).await?;
            // Incremental sync: Lichess filters by `since` (ms), overlapping 60 s for skew.
            // It returns newest first and caps each page, so the walk pages backwards with
            // `until`; a long gap between syncs must not silently lose games.
            let since = row.last_synced_at.map(|at| at - SYNC_OVERLAP_MS);
            let mut fetched = 0usize;
            let mut until: Option<i64> = None;
            let mut downgraded = false;
            while fetched < MAX_GAMES {
                let (games, reviews, settled, page_downgraded) =
                    self.fetch_games_page(name, since, until).await?;
                downgraded |= page_downgraded;
                if !current() {
                    return Err(sync_cancelled());
                }
                if !games.is_empty() {
                    self.store
                        .save_games_page(name, &games, &reviews, &current)
                        .await?;
                }
                if !current() {
                    return Err(sync_cancelled());
                }
                self.store.mark_checked(&settled, &current).await?;
                fetched += games.len();
                if games.len() < SYNC_PAGE {
                    break;
                }
                let oldest = games.iter().map(|game| game.created_at).min().unwrap_or(0);
                if until.is_some_and(|bound| oldest >= bound) {
                    break;
                }
                until = Some(oldest);
            }
            // A 401-downgraded sync yields a successful partial (public games only):
            // do not advance the cursor, so the gap is backfilled after reconnect. A tracked
            // player has no login, so public games are the whole sync: holding its cursor
            // back re-downloaded its full history every time.
            if !downgraded || !row.connected {
                self.store
                    .save_games_cursor(name, started_at, &current)
                    .await?;
                if let Some(play_time) = play_time {
                    self.store
                        .save_synced_play_time(name, play_time, &current)
                        .await?;
                }
            } else {
                self.host.log(
                    Level::Warn,
                    "lichess",
                    &format!("Game sync for {name} used public games only; cursor unchanged."),
                );
            }
            self.forget_profile(name);
            Ok::<usize, CoreError>(fetched)
        }
        .await
    }

    /// One page of the sync: its games, Lichess's reviews of them, and the finished games it
    /// has settled (reviewed, or old enough that Lichess has had its chance to analyse them).
    async fn fetch_games_page(
        &self,
        account: &str,
        since: Option<i64>,
        until: Option<i64>,
    ) -> CoreResult<(Vec<LichessGame>, Vec<StoredReview>, Vec<String>, bool)> {
        let mut query = vec![
            ("max", SYNC_PAGE.to_string()),
            ("ongoing", "true".to_string()),
        ];
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        if let Some(until) = until {
            query.push(("until", until.to_string()));
        }
        for flag in [
            "opening",
            "moves",
            "pgnInJson",
            "clocks",
            "evals",
            "accuracy",
        ] {
            query.push((flag, "true".to_string()));
        }
        let path = format!("/api/games/user/{}", path_segment(account));
        let (records, downgraded) = self
            .collect_records(account, reqwest::Method::GET, &path, &query, None)
            .await?;
        let mut games = Vec::with_capacity(records.len());
        let mut reviews = Vec::new();
        let mut settled = Vec::new();
        let now = now_ms();
        for value in &records {
            let raw = read_game(value)?;
            games.push(normalize_game(&raw, account));
            let review = review_from_lichess(value);
            if !is_game_in_progress(value)
                && (review.is_some() || now - raw.last_move_at > ANALYSIS_SETTLED_MS)
            {
                settled.push(raw.id.clone());
            }
            reviews.extend(review);
        }
        Ok((games, reviews, settled, downgraded))
    }

    /// Unfinished games are re-fetched by ID, even when their creation predates the next window.
    async fn refresh_pending_games(
        &self,
        account: &str,
        current: &(dyn Fn() -> bool + Sync),
    ) -> CoreResult<()> {
        let ids = self.store.pending_game_ids(account).await?;
        let query: Vec<(&str, String)> = [
            "moves",
            "pgnInJson",
            "clocks",
            "opening",
            "evals",
            "accuracy",
        ]
        .into_iter()
        .map(|flag| (flag, "true".to_string()))
        .collect();
        for chunk in ids.chunks(300) {
            let requested: std::collections::HashSet<&str> =
                chunk.iter().map(String::as_str).collect();
            let (records, _) = self
                .collect_records(
                    account,
                    reqwest::Method::POST,
                    "/api/games/export/_ids",
                    &query,
                    Some(chunk.join(",")),
                )
                .await?;
            let mut games = Vec::new();
            let mut reviews = Vec::new();
            for value in &records {
                let raw = read_game(value)?;
                if requested.contains(raw.id.as_str()) {
                    games.push(normalize_game(&raw, account));
                    reviews.extend(review_from_lichess(value));
                }
            }
            // A missing ID stays pending; a failed request never moves the cursor.
            self.store
                .save_games_page(account, &games, &reviews, current)
                .await?;
        }
        Ok(())
    }

    /// The game records of an NDJSON response for `account`, read with its own token when it
    /// has one. Returns whether the request was downgraded to
    /// anonymous (missing/refused login).
    async fn collect_records(
        &self,
        account: &str,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<String>,
    ) -> CoreResult<(Vec<Value>, bool)> {
        let (lines, downgraded) = self
            .as_owner_downgraded(account, UsageKind::Games, |auth| {
                let method = method.clone();
                let body = body.clone();
                async move {
                    let mut request = self
                        .client
                        .request(method, path, &auth)
                        .query(query)
                        .lane(Lane::Export);
                    if let Some(body) = body {
                        request = request.text(body);
                    }
                    request.records("game", usize::MAX, Some::<Value>).await
                }
            })
            .await?;
        Ok((lines, downgraded))
    }
}

fn sync_cancelled() -> CoreError {
    CoreError::new("Sync was cancelled.")
}

/// Games that ended this long ago have had their chance to be analysed on Lichess.
const ANALYSIS_SETTLED_MS: i64 = 86_400_000;

/// Whether a game record is still being played, by its `status`.
fn is_game_in_progress(raw: &Value) -> bool {
    kchess_domain::records::games::is_game_in_progress(raw.get("status"))
}
