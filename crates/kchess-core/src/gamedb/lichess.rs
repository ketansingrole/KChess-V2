//! Strong Lichess games: a month of Lichess's rated games (`database.lichess.org/standard`, CC0,
//! one `.pgn.zst` file a month of some 30 GB and 90 million games), of which only the games whose
//! players were both rated at least a floor are kept, bullet and bots' games left out. Each floor
//! has a database of its own (`lichess-2400`, "Lichess 2400+") that gathers months, as the
//! broadcast games do: each import records the file it read.
//!
//! A month is far too large to save first: it is read as it downloads. The download sends its
//! bytes to the import's thread (`Arriving`), which decodes them as it reads a `.pgn.zst` file
//! and passes over every game the floor leaves out by its header lines alone
//! (`import::HeaderFilter`), so only about one game in a hundred is parsed. The bytes are hashed
//! as they arrive; a digest other than the archive's, a lost connection or Cancel reaches the
//! import as a failed read, so it ends as a failed import does: the month's games are removed,
//! and the database stays as it was.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use super::master::{SILENCE, archive_months, fetch_text, installed, month_of};
use super::{GameDbService, Reporter, TREE_ROWS, failed, import, progress};
use crate::download;
use crate::usage::Meter;
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    DatabaseSource, GameDbImported, GameDbPhase as Phase, LichessArchive, LichessGames,
    LichessHeld, UsageKind,
};

/// The archive: `list.txt`, `sha256sums.txt` and the months beside them.
pub const ARCHIVE: &str = crate::sources::LICHESS_STANDARD.url;
const PREFIX: &str = "lichess_db_standard_rated_";
/// The most one month may bring: the largest was some 33 GB (2026).
const MAX_MONTH: u64 = 128 * 1024 * 1024 * 1024;
/// How long Lichess may take to say how large a month is.
const SIZE_TIMEOUT: Duration = Duration::from_secs(30);
/// Chunks of the download held for the import at most: a few MB.
const AHEAD: usize = 64;

const ID_PREFIX: &str = "lichess-";

/// The database of the strong games of floor `min_rating`.
pub fn id(min_rating: u32) -> String {
    format!("{ID_PREFIX}{min_rating}")
}

/// Its name as first listed; the person may rename it.
pub fn name(min_rating: u32) -> String {
    format!("Lichess {min_rating}+")
}

/// The floor of the database `id`, when it is one of these.
pub fn rating_of(id: &str) -> Option<u32> {
    id.strip_prefix(ID_PREFIX)?.parse().ok()
}

/// The file of a month.
fn file_of(month: &str) -> String {
    format!("{PREFIX}{month}.pgn.zst")
}

/// The speed Lichess gives a time control (`180+2`, or `-` for correspondence), by its estimated
/// duration: the base time plus forty increments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speed {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

fn speed(time_control: &str) -> Option<Speed> {
    if time_control == "-" {
        return Some(Speed::Correspondence);
    }
    let (base, increment) = time_control.split_once('+')?;
    let duration = base.parse::<u64>().ok()? + 40 * increment.parse::<u64>().ok()?;
    Some(match duration {
        0..30 => Speed::UltraBullet,
        30..180 => Speed::Bullet,
        180..480 => Speed::Blitz,
        480..1500 => Speed::Rapid,
        _ => Speed::Classical,
    })
}

/// Whether a game with these header lines is one of the strong games of floor `min_rating`:
/// both players rated at least that, neither a bot, and slower than bullet.
pub fn strong(headers: &[u8], min_rating: u32) -> bool {
    let (mut white, mut black, mut speed_of, mut bot) = (None, None, None, false);
    for line in headers.split(|&b| b == b'\n') {
        let Some(line) = std::str::from_utf8(line).ok() else {
            continue;
        };
        let Some((key, value)) = line
            .trim()
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
            .and_then(|line| line.split_once(' '))
        else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key {
            "WhiteElo" => white = value.parse::<u32>().ok(),
            "BlackElo" => black = value.parse::<u32>().ok(),
            "TimeControl" => speed_of = speed(value),
            "WhiteTitle" | "BlackTitle" => bot |= value == "BOT",
            _ => {}
        }
    }
    let rated = |elo: Option<u32>| elo.is_some_and(|elo| elo >= min_rating);
    rated(white)
        && rated(black)
        && !bot
        && speed_of.is_some_and(|speed| !matches!(speed, Speed::UltraBullet | Speed::Bullet))
}

/// The download's bytes as the import reads them, chunk by chunk; a failed download is a failed
/// read. It cannot seek: it is read as a `.pgn.zst` file, front to back.
struct Arriving {
    receive: tokio::sync::mpsc::Receiver<io::Result<bytes::Bytes>>,
    chunk: bytes::Bytes,
}

impl Read for Arriving {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.chunk.is_empty() {
            match self.receive.blocking_recv() {
                Some(chunk) => self.chunk = chunk?,
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.chunk.len());
        out[..n].copy_from_slice(&self.chunk.split_to(n));
        Ok(n)
    }
}

impl Seek for Arriving {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a download is read front to back",
        ))
    }
}

/// What the import tells along the way: only that the tree is being built. The download reports
/// its own progress, as it knows the bytes received.
struct Quiet {
    reporter: Reporter,
}

impl import::Watch for Quiet {
    fn reading(&mut self, _: import::Count) {}

    fn indexing(&mut self, count: import::Count) {
        self.reporter.send(Phase::Indexing, count, true);
    }

    fn cancelled(&self) -> bool {
        self.reporter.cancel.is_cancelled()
    }
}

/// How the Lichess games download words its failures.
const WORDING: download::Wording = download::Wording {
    network: |error| {
        if error.is_timeout() {
            "The Lichess games download timed out.".to_string()
        } else {
            format!("Could not download the Lichess games: {error}")
        }
    },
    status: |status| format!("Lichess answered {status} for its games."),
    too_large: "The Lichess games download is unexpectedly large.",
};

fn cancelled() -> CoreError {
    CoreError::new("Download cancelled.")
}

impl GameDbService {
    /// The archive's months (`archive`: `ARCHIVE`, or a test's), newest first, with a digest
    /// each, and the months each database of strong Lichess games holds.
    pub async fn lichess_months(&self, archive: &str, meter: &dyn Meter) -> Result<LichessArchive> {
        let client = download::client(Some(SILENCE))?;
        let cancel = CancellationToken::new();
        let usage = download::Usage {
            meter,
            kind: UsageKind::Database,
        };
        let list = format!("{archive}list.txt");
        let list = fetch_text(&client, &list, &cancel, usage, &WORDING).await?;
        let sums = format!("{archive}sha256sums.txt");
        let sums = fetch_text(&client, &sums, &cancel, usage, &WORDING).await?;
        let (months, _) = archive_months(&list, &sums, PREFIX);
        let mut held = Vec::new();
        for info in self.status().await?.databases {
            let Some(min_rating) = info.lichess_rating else {
                continue;
            };
            let files = self.inner.reader(&info.id)?.run(installed).await?;
            let mut months: Vec<String> = files
                .iter()
                .filter_map(|file| month_of(file, PREFIX).map(str::to_string))
                .collect();
            months.sort_unstable_by(|a, b| b.cmp(a));
            held.push(LichessHeld { min_rating, months });
        }
        held.sort_by_key(|held| held.min_rating);
        Ok(LichessArchive {
            months: months.into_iter().rev().map(|month| month.month).collect(),
            held,
        })
    }

    /// How many bytes `month` downloads, when Lichess says.
    pub async fn lichess_size(
        &self,
        archive: &str,
        month: &str,
        meter: &dyn Meter,
    ) -> Result<Option<u64>> {
        let client = download::client(Some(SILENCE))?;
        let url = format!("{archive}{}", file_of(month));
        let sent = client.head(&url).timeout(SIZE_TIMEOUT).send().await;
        meter.count("", UsageKind::Database, 1, 0);
        let response = sent.map_err(|cause| CoreError::new((WORDING.network)(&cause)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::new(format!("Lichess has no games for {month}.")));
        }
        if !response.status().is_success() {
            return Err(CoreError::new((WORDING.status)(response.status().as_u16())));
        }
        // A HEAD's body is empty: the length is the file's, from its header.
        Ok(response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse().ok())
            .filter(|&bytes: &u64| bytes > 0))
    }

    /// Downloads the month `request` names (or the newest the floor's database lacks) from
    /// `archive` and keeps its strong games in that database. Progress arrives as
    /// `gamedb:progress`; `cancel` stops it and keeps the database as it was.
    pub async fn lichess_install(
        &self,
        archive: &str,
        request: LichessGames,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let result = self.lichess_install_inner(archive, request, meter).await;
        self.inner.logged("Lichess games download", result)
    }

    async fn lichess_install_inner(
        &self,
        archive: &str,
        request: LichessGames,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let id = id(request.min_rating);
        let mut guard = self.claim(&id)?;
        let running = Arc::clone(&guard.running);
        let count = match self
            .bring_month(&id, archive, &request, meter, &running.cancel)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                let error = if running.cancel.is_cancelled() {
                    cancelled()
                } else {
                    error
                };
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

    async fn bring_month(
        &self,
        id: &str,
        archive: &str,
        request: &LichessGames,
        meter: &dyn Meter,
        cancel: &CancellationToken,
    ) -> Result<import::Count> {
        let inner = &self.inner;
        let name = name(request.min_rating);
        let client = download::client(Some(SILENCE))?;
        let usage = download::Usage {
            meter,
            kind: UsageKind::Database,
        };
        let mut reporter = Reporter::new(&inner.host, id, cancel);
        reporter.send(Phase::Downloading, import::Count::default(), true);
        let list = format!("{archive}list.txt");
        let list = fetch_text(&client, &list, cancel, usage, &WORDING).await?;
        let sums = format!("{archive}sha256sums.txt");
        let sums = fetch_text(&client, &sums, cancel, usage, &WORDING).await?;
        let (months, _) = archive_months(&list, &sums, PREFIX);
        let file = inner.file(id);
        let held = if file.is_file() {
            inner.reader(id)?.run(installed).await?
        } else {
            HashSet::new()
        };
        let month = match &request.month {
            Some(wanted) => {
                let month = months
                    .iter()
                    .find(|month| &month.month == wanted)
                    .ok_or_else(|| {
                        CoreError::new(format!("Lichess has no games for {wanted} yet."))
                    })?;
                if held.contains(&month.file) {
                    return Err(CoreError::new(format!("{name} already has {wanted}.")));
                }
                month
            }
            None => months
                .iter()
                .rev()
                .find(|month| !held.contains(&month.file))
                .ok_or_else(|| CoreError::new(format!("{name} has every month.")))?,
        }
        .clone();
        std::fs::create_dir_all(&inner.dir)
            .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
        reporter.month = Some(month.month.clone());
        let url = format!("{archive}{}", month.file);
        let limits = download::Limits {
            max_bytes: MAX_MONTH,
            deadline: None,
        };
        let mut body = download::open(
            &client,
            || client.get(&url),
            limits,
            cancel,
            usage,
            &WORDING,
        )
        .await?;
        let total_bytes = body.total().unwrap_or(0);
        let (send, receive) = tokio::sync::mpsc::channel(AHEAD);
        let kept = Arc::new(AtomicU64::new(0));
        let keep: import::Keep = {
            let (kept, floor) = (Arc::clone(&kept), request.min_rating);
            Box::new(move |headers| {
                let strong = strong(headers, floor);
                if strong {
                    kept.fetch_add(1, Ordering::Relaxed);
                }
                strong
            })
        };
        let input = import::Input {
            reader: Arriving {
                receive,
                chunk: bytes::Bytes::new(),
            },
            total_bytes,
            packing: PgnPacking::Zstd,
            label: Some(month.file.clone()),
            keep: Some(keep),
            entry: None,
        };
        let target = if file.is_file() {
            import::Target::Existing { file: file.clone() }
        } else {
            inner.new_target(id, name.clone(), DatabaseSource::Lichess, None)
        };
        let mut watch = Quiet {
            reporter: Reporter::new(&inner.host, id, cancel),
        };
        watch.reporter.month = Some(month.month.clone());
        let (dir, host) = (inner.dir.clone(), Arc::clone(&inner.host));
        let reading = tokio::task::spawn_blocking(move || {
            let result = import::read_into(input, &target, &dir, TREE_ROWS, &mut watch);
            if let import::Target::New { file, .. } = &target
                && result.is_ok()
            {
                crate::store::restrict_database(file, host.as_ref(), "gamedb");
            }
            result
        });
        let downloading = async {
            // Held by the download alone: the import reads to the end once it is dropped.
            let send = send;
            let mut hasher = Sha256::new();
            let mut count = import::Count {
                total_bytes,
                ..import::Count::default()
            };
            reporter.send(Phase::Downloading, count, true);
            loop {
                let next = tokio::select! {
                    next = body.next() => next,
                    // The import stopped (its own failure): it says why.
                    () = send.closed() => return Ok(()),
                };
                let chunk = match next {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(error) => {
                        send.send(Err(io::Error::other("the download failed")))
                            .await
                            .ok();
                        return Err(error);
                    }
                };
                hasher.update(&chunk);
                // The import stopped (its own failure): it says why.
                if send.send(Ok(chunk)).await.is_err() {
                    return Ok(());
                }
                count.bytes = body.received();
                count.games = kept.load(Ordering::Relaxed);
                reporter.send(Phase::Downloading, count, false);
            }
            if kchess_delta::hex(&hasher.finalize()) != month.sha256 {
                send.send(Err(io::Error::other("the download is not the archive's")))
                    .await
                    .ok();
                return Err(CoreError::new(format!(
                    "The Lichess games of {} did not arrive intact. Try again.",
                    month.month
                )));
            }
            Ok(())
        };
        let (downloaded, read) = tokio::join!(downloading, reading);
        // The download's failure is the cause of the import's.
        downloaded?;
        read.map_err(|_| CoreError::new("The import stopped. Try again."))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(white: &str, black: &str, control: &str, title: Option<&str>) -> Vec<u8> {
        let mut text = format!(
            "[Event \"Rated Blitz game\"]\n[WhiteElo \"{white}\"]\n[BlackElo \"{black}\"]\n\
             [TimeControl \"{control}\"]\n"
        );
        if let Some(title) = title {
            text.push_str(&format!("[BlackTitle \"{title}\"]\n"));
        }
        text.into_bytes()
    }

    #[test]
    fn a_time_control_has_lichess_s_speed() {
        assert_eq!(speed("15+0"), Some(Speed::UltraBullet));
        assert_eq!(speed("60+0"), Some(Speed::Bullet));
        // Two minutes and one second a move: 120 + 40 = 160 seconds.
        assert_eq!(speed("120+1"), Some(Speed::Bullet));
        assert_eq!(speed("180+0"), Some(Speed::Blitz));
        assert_eq!(speed("300+3"), Some(Speed::Blitz));
        assert_eq!(speed("600+0"), Some(Speed::Rapid));
        assert_eq!(speed("1800+0"), Some(Speed::Classical));
        assert_eq!(speed("-"), Some(Speed::Correspondence));
        assert_eq!(speed("?"), None);
    }

    #[test]
    fn a_strong_game_has_two_rated_players_no_bot_and_no_bullet() {
        assert!(strong(&headers("2450", "2400", "180+0", None), 2400));
        assert!(strong(&headers("2400", "2401", "-", None), 2400));
        assert!(strong(&headers("2400", "2401", "600+5", Some("GM")), 2400));
        assert!(!strong(&headers("2450", "2399", "180+0", None), 2400));
        assert!(!strong(&headers("?", "2500", "180+0", None), 2400));
        assert!(!strong(&headers("2600", "2600", "60+0", None), 2400));
        assert!(!strong(
            &headers("2600", "2600", "180+0", Some("BOT")),
            2400
        ));
        assert!(!strong(b"[WhiteElo \"2600\"]\n[BlackElo \"2600\"]\n", 2400));
    }

    #[test]
    fn a_floor_names_its_database() {
        assert_eq!(id(2400), "lichess-2400");
        assert_eq!(name(2400), "Lichess 2400+");
        assert_eq!(rating_of("lichess-2400"), Some(2400));
        assert_eq!(rating_of("master"), None);
        assert!(kchess_contracts::check::is_database_id(&id(2400)));
    }
}
