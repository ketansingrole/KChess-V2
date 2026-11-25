//! One player's games, downloaded from Lichess or Chess.com into a database of their own, to
//! prepare against them: every game, or those of the newest months, of some speeds, rated only.
//!
//! The database's id comes from the site and the player (`id`), so a second download of the same
//! player replaces their games in place (`import::Target::Replace`): a search sees the old games
//! or the new ones, never a mix. The games arrive as PGN into a `.partial` file in the databases
//! folder, counted as they come (`Sink`), and are read in once all have arrived; a failure or a
//! cancellation removes the download and leaves the database as it was. The request is kept with
//! the database (`schema::set_player`), so an update asks for the same games again.

use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use super::{GameDbService, Reporter, failed, import, progress, schema};
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    DatabaseSource, ExplorerSpeed, GameDbImported, GameDbPhase as Phase, GameSite, PlayerGames,
};
use kchess_domain::calendar::{civil_from_days, days_from_civil};

/// The speeds of a standard game on Lichess: asked for when the request names none, so that
/// variants' games are not downloaded only to be skipped.
const LICHESS_SPEEDS: [ExplorerSpeed; 6] = [
    ExplorerSpeed::UltraBullet,
    ExplorerSpeed::Bullet,
    ExplorerSpeed::Blitz,
    ExplorerSpeed::Rapid,
    ExplorerSpeed::Classical,
    ExplorerSpeed::Correspondence,
];

/// The database of `username`'s games on `site`: the site, then a digest of the name as the site
/// compares it (without case), so every name, underscores and all, makes an id of the same form.
pub fn id(site: GameSite, username: &str) -> String {
    let digest = Sha256::digest(format!("{}:{}", site.as_str(), username.to_lowercase()));
    format!("{}-{}", site.as_str(), &kchess_delta::hex(&digest)[..12])
}

/// The database's name as first listed: the player and the site.
pub fn name(site: GameSite, username: &str) -> String {
    let site = match site {
        GameSite::Lichess => "Lichess",
        GameSite::Chesscom => "Chess.com",
    };
    format!("{username} · {site}")
}

/// The first day (UTC) of the oldest of the newest `months` months at `now_ms`, the current month
/// among them: `(year, month)`.
fn first_month(months: u32, now_ms: i64) -> (i64, i64) {
    let (year, month, _) = civil_from_days(now_ms.div_euclid(86_400_000)).unwrap_or((1970, 1, 1));
    let index = year * 12 + (month - 1) - (i64::from(months) - 1);
    (index.div_euclid(12), index.rem_euclid(12) + 1)
}

/// What Lichess's export of a user's games is asked (`/api/games/user/{name}`): their standard
/// games of the speeds asked, rated only when asked, from the first month asked, with their
/// opening named and without clocks or analysis.
pub fn lichess_query(request: &PlayerGames, now_ms: i64) -> Vec<(&'static str, String)> {
    let speeds = request.speeds.as_deref().unwrap_or(&LICHESS_SPEEDS);
    let speeds: Vec<&str> = speeds.iter().map(|speed| speed.as_str()).collect();
    let mut query = vec![
        ("perfType", speeds.join(",")),
        ("moves", "true".to_string()),
        ("tags", "true".to_string()),
        ("opening", "true".to_string()),
        ("clocks", "false".to_string()),
        ("evals", "false".to_string()),
        ("literate", "false".to_string()),
    ];
    if request.rated {
        query.push(("rated", "true".to_string()));
    }
    if let Some(months) = request.months {
        let (year, month) = first_month(months, now_ms);
        let since = days_from_civil(year, month, 1).unwrap_or(0) * 86_400_000;
        query.push(("since", since.to_string()));
    }
    query
}

/// The first of Chess.com's monthly archives (`YYYY/MM`) the request asks for; every one when it
/// names no months.
pub fn chesscom_from(request: &PlayerGames, now_ms: i64) -> Option<String> {
    let (year, month) = first_month(request.months?, now_ms);
    Some(format!("{year:04}/{month:02}"))
}

/// Whether a game of a Chess.com archive is one the request asks for: standard chess of a speed
/// asked (Chess.com's rapid stands for rapid and classical, daily for correspondence), rated
/// when asked.
pub fn chesscom_keeps(request: &PlayerGames, game: &Value) -> bool {
    let text = |key| game.get(key).and_then(Value::as_str);
    if text("rules") != Some("chess") || text("pgn").is_none_or(|pgn| pgn.trim().is_empty()) {
        return false;
    }
    if request.rated && game.get("rated").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let Some(speeds) = &request.speeds else {
        return true;
    };
    let matching: &[ExplorerSpeed] = match text("time_class") {
        Some("bullet") => &[ExplorerSpeed::UltraBullet, ExplorerSpeed::Bullet],
        Some("blitz") => &[ExplorerSpeed::Blitz],
        Some("rapid") => &[ExplorerSpeed::Rapid, ExplorerSpeed::Classical],
        Some("daily") => &[ExplorerSpeed::Correspondence],
        _ => &[],
    };
    matching.iter().any(|speed| speeds.contains(speed))
}

/// Where a download writes the games it receives: the `.partial` file they are read from once
/// all have arrived, its games counted as they come and reported to every window.
pub struct Sink {
    file: BufWriter<File>,
    reporter: Reporter,
    count: import::Count,
    /// The end of the last text written, which the next may continue: a game's first tag split
    /// between two pieces is still counted once.
    tail: Vec<u8>,
}

/// How a game's text starts in a PGN export, at the start of a line.
const GAME_START: &[u8] = b"[Event ";

impl Sink {
    fn new(file: File, reporter: Reporter) -> Sink {
        Sink {
            file: BufWriter::new(file),
            reporter,
            count: import::Count::default(),
            // A game at the very start of the text starts a line too.
            tail: b"\n".to_vec(),
        }
    }

    fn saved(cause: std::io::Error) -> CoreError {
        CoreError::wrap("Could not save the player's games", cause)
    }

    /// A piece of a PGN text as it arrives (a network read's worth), on disk once it returns:
    /// its games are counted by their first tag.
    pub fn pgn(&mut self, text: &str) -> Result<()> {
        self.file.write_all(text.as_bytes()).map_err(Self::saved)?;
        self.file.flush().map_err(Self::saved)?;
        let mut seen = std::mem::take(&mut self.tail);
        seen.extend_from_slice(text.as_bytes());
        let starts = seen
            .windows(GAME_START.len() + 1)
            .filter(|window| window[0] == b'\n' && &window[1..] == GAME_START)
            .count();
        self.count.games += starts as u64;
        let keep = seen.len().min(GAME_START.len());
        self.tail = seen.split_off(seen.len() - keep);
        self.count.bytes += text.len() as u64;
        self.reporter.send(Phase::Downloading, self.count, false);
        Ok(())
    }

    /// One game's whole PGN.
    pub fn game(&mut self, pgn: &str) -> Result<()> {
        let pgn = pgn.trim();
        for piece in [pgn.as_bytes(), b"\n\n"] {
            self.file.write_all(piece).map_err(Self::saved)?;
        }
        self.count.games += 1;
        self.count.bytes += pgn.len() as u64 + 2;
        self.reporter.send(Phase::Downloading, self.count, false);
        Ok(())
    }

    /// Games received so far.
    pub fn games(&self) -> u64 {
        self.count.games
    }

    /// Cancelled when the download should stop: Cancel, or the core closing.
    pub fn cancel(&self) -> CancellationToken {
        self.reporter.cancel.clone()
    }

    /// Every game written down, and the reporter back for the import.
    fn finish(self) -> Result<(u64, Reporter)> {
        let Sink {
            file,
            reporter,
            count,
            ..
        } = self;
        let file = file.into_inner().map_err(|e| Self::saved(e.into_error()))?;
        file.sync_all().map_err(Self::saved)?;
        Ok((count.games, reporter))
    }
}

/// The download, removed however it ends.
struct Partial(PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        // Already gone when the download never started; `sweep` removes what is left over.
        std::fs::remove_file(&self.0).ok();
    }
}

impl GameDbService {
    /// Downloads the games `request` names into its player's database: `fetch` writes them to
    /// the sink it is given as they arrive and gives it back once all have. Progress arrives as
    /// `gamedb:progress`; Cancel stops it, leaving the database as it was.
    pub async fn player_install<F, Fut>(
        &self,
        request: PlayerGames,
        fetch: F,
    ) -> Result<GameDbImported>
    where
        F: FnOnce(Sink) -> Fut,
        Fut: Future<Output = Result<Sink>>,
    {
        let result = self.player_install_inner(request, fetch).await;
        self.inner.logged("player download", result)
    }

    async fn player_install_inner<F, Fut>(
        &self,
        request: PlayerGames,
        fetch: F,
    ) -> Result<GameDbImported>
    where
        F: FnOnce(Sink) -> Fut,
        Fut: Future<Output = Result<Sink>>,
    {
        let id = id(request.site, &request.username);
        let mut guard = self.claim(&id)?;
        let running = Arc::clone(&guard.running);
        let count = match self
            .bring_player(&id, &request, &running.cancel, fetch)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                guard.end = Some(failed(&running, &id, &error));
                return Err(error);
            }
        };
        let database =
            self.inner.info(&id).await?.ok_or_else(|| {
                CoreError::new("The database could not be read after the download.")
            })?;
        guard.end = Some(progress(Phase::Done, &id, count, None));
        drop(guard);
        Ok(GameDbImported {
            database,
            added: count.games,
            skipped: count.skipped,
        })
    }

    async fn bring_player<F, Fut>(
        &self,
        id: &str,
        request: &PlayerGames,
        cancel: &CancellationToken,
        fetch: F,
    ) -> Result<import::Count>
    where
        F: FnOnce(Sink) -> Fut,
        Fut: Future<Output = Result<Sink>>,
    {
        let inner = &self.inner;
        std::fs::create_dir_all(&inner.dir)
            .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
        let partial = Partial(inner.dir.join(format!("{id}.pgn.partial")));
        let file = File::create(&partial.0).map_err(Sink::saved)?;
        let mut reporter = Reporter::new(&inner.host, id, cancel);
        reporter.send(Phase::Downloading, import::Count::default(), true);
        let fetched = fetch(Sink::new(file, reporter)).await;
        // However the site's request ended once Cancel came, the download was cancelled.
        if cancel.is_cancelled() {
            return Err(CoreError::new("Download cancelled."));
        }
        let sink = fetched?;
        let (games, mut reporter) = sink.finish()?;
        if games == 0 {
            return Err(CoreError::new(format!(
                "{} has no games that match.",
                request.username
            )));
        }
        let file = inner.file(id);
        let target = if file.is_file() {
            import::Target::Replace { file: file.clone() }
        } else {
            inner.new_target(
                id,
                name(request.site, &request.username),
                DatabaseSource::Player,
                None,
            )
        };
        reporter.send(Phase::Reading, import::Count::default(), true);
        let (path, label) = (
            partial.0.clone(),
            format!("{}:{}", request.site.as_str(), request.username),
        );
        let count = inner
            .run_import(target, PgnPacking::Plain, reporter, move || {
                import::Source::downloaded(&path, &label)
            })
            .await?;
        drop(partial);
        let kept = request.clone();
        tokio::task::spawn_blocking(move || {
            let db = schema::open_write(&file)?;
            schema::set_player(&db, &kept)
        })
        .await
        .map_err(|_| CoreError::new("Game database stopped. Retry the operation."))??;
        // The last position search's games may be this database's old ones.
        inner.listed.forget();
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(
        months: Option<u32>,
        speeds: Option<Vec<ExplorerSpeed>>,
        rated: bool,
    ) -> PlayerGames {
        PlayerGames {
            site: GameSite::Lichess,
            username: "Some_Player".to_string(),
            months,
            speeds,
            rated,
        }
    }

    /// 2026-11-25 (UTC).
    const NOW: i64 = 1_795_564_800_000;

    #[test]
    fn a_players_database_is_named_by_site_and_name_without_case() {
        let lichess = id(GameSite::Lichess, "Some_Player");
        assert_eq!(lichess, id(GameSite::Lichess, "some_player"));
        assert_ne!(lichess, id(GameSite::Chesscom, "Some_Player"));
        assert_ne!(lichess, id(GameSite::Lichess, "Some-Player"));
        assert!(lichess.starts_with("lichess-"));
        assert!(kchess_contracts::check::is_database_id(&lichess));
        assert!(kchess_contracts::check::is_database_id(&id(
            GameSite::Chesscom,
            "a"
        )));
        assert_eq!(name(GameSite::Chesscom, "Hikaru"), "Hikaru · Chess.com");
    }

    #[test]
    fn months_count_back_from_the_current_one() {
        assert_eq!(first_month(1, NOW), (2026, 11));
        assert_eq!(first_month(11, NOW), (2026, 1));
        assert_eq!(first_month(12, NOW), (2025, 12));
        assert_eq!(first_month(600, NOW), (1976, 12));
        assert_eq!(
            chesscom_from(&request(Some(3), None, false), NOW).as_deref(),
            Some("2026/09")
        );
        assert_eq!(chesscom_from(&request(None, None, false), NOW), None);
    }

    #[test]
    fn lichess_is_asked_for_the_standard_games_the_request_names() {
        let query = |request: &PlayerGames| -> Vec<(String, String)> {
            lichess_query(request, NOW)
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect()
        };
        let get = |query: &[(String, String)], key: &str| {
            query.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        };
        let every = query(&request(None, None, false));
        assert_eq!(
            get(&every, "perfType").as_deref(),
            Some("ultraBullet,bullet,blitz,rapid,classical,correspondence")
        );
        assert_eq!(get(&every, "rated"), None);
        assert_eq!(get(&every, "since"), None);
        assert_eq!(get(&every, "clocks").as_deref(), Some("false"));

        let some = query(&request(
            Some(12),
            Some(vec![ExplorerSpeed::Blitz, ExplorerSpeed::Rapid]),
            true,
        ));
        assert_eq!(get(&some, "perfType").as_deref(), Some("blitz,rapid"));
        assert_eq!(get(&some, "rated").as_deref(), Some("true"));
        // 2025-12-01T00:00:00Z.
        assert_eq!(get(&some, "since").as_deref(), Some("1764547200000"));
    }

    #[test]
    fn chesscom_games_are_kept_by_rules_speed_and_rating() {
        let game = |rules: &str, class: &str, rated: bool| json!({ "rules": rules, "time_class": class, "rated": rated, "pgn": "1. e4 *" });
        let every = request(None, None, false);
        assert!(chesscom_keeps(&every, &game("chess", "daily", false)));
        assert!(!chesscom_keeps(&every, &game("chess960", "blitz", true)));
        assert!(!chesscom_keeps(
            &every,
            &json!({ "rules": "chess", "time_class": "blitz" })
        ));
        let slow = request(
            None,
            Some(vec![
                ExplorerSpeed::Classical,
                ExplorerSpeed::Correspondence,
            ]),
            true,
        );
        assert!(chesscom_keeps(&slow, &game("chess", "rapid", true)));
        assert!(chesscom_keeps(&slow, &game("chess", "daily", true)));
        assert!(!chesscom_keeps(&slow, &game("chess", "rapid", false)));
        assert!(!chesscom_keeps(&slow, &game("chess", "blitz", true)));
    }
}
