//! The master games: Lichess's archive of over-the-board broadcast games
//! (`database.lichess.org/broadcast`, CC BY-SA 4.0), one `.pgn.zst` file a month, brought into
//! one database (`master`) a month at a time.
//!
//! `list.txt` names the monthly files and `sha256sums.txt` their digests. Each month the database
//! lacks is downloaded to a `.partial` file in the databases folder, checked against its digest,
//! then read in: the first month makes the database, later ones are added to it. Each import
//! records the file it read (`imports.file`), so the database itself says which months it holds,
//! and a later run (the monthly top-up) brings only the others. A failure or a cancellation
//! removes that month's download and keeps the months already in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use super::{GameDbService, Reporter, Running, failed, import, progress};
use crate::download;
use crate::host::Level;
use crate::store::sql::Sql as _;
use crate::usage::Meter;
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{DatabaseSource, GameDbImported, GameDbPhase as Phase, UsageKind};

/// The master games database's id.
pub const ID: &str = "master";
/// Its name as first listed; the person may rename it.
pub const NAME: &str = "Master games";
/// Whose games they are and on what terms, as the licence asks to be shown.
pub const ATTRIBUTION: &str = "Lichess broadcast games, database.lichess.org, CC BY-SA 4.0";
/// The archive: `list.txt`, `sha256sums.txt` and the months beside them.
pub const ARCHIVE: &str = "https://database.lichess.org/broadcast/";

/// How long a download may go without a byte before it fails.
pub(super) const SILENCE: Duration = Duration::from_secs(60);
/// The most one month may bring: the largest was some 31 MB (2026-08).
const MAX_MONTH: u64 = 1024 * 1024 * 1024;
/// The most `list.txt` or `sha256sums.txt` may bring: some 8 KB each (2026-11).
const MAX_INDEX: u64 = 1024 * 1024;
/// How long each of those two files may take, in all.
const INDEX_TIMEOUT: Duration = Duration::from_secs(60);

const PREFIX: &str = "lichess_db_broadcast_";
const SUFFIX: &str = ".pgn.zst";

/// One month of an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Month {
    /// `YYYY-MM`.
    pub month: String,
    /// The file's name (`lichess_db_broadcast_YYYY-MM.pgn.zst`), as its import records it.
    pub file: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
}

/// The month a file of an archive of Lichess's (`database.lichess.org`) holds (`2026-10`),
/// when `file` is one of its files: `prefix`, the month, then `.pgn.zst`.
pub(super) fn month_of<'a>(file: &'a str, prefix: &str) -> Option<&'a str> {
    let month = file.strip_prefix(prefix)?.strip_suffix(SUFFIX)?;
    let bytes = month.as_bytes();
    let valid = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(at, b)| at == 4 || b.is_ascii_digit())
        && matches!(month[5..].parse::<u8>(), Ok(1..=12));
    valid.then_some(month)
}

/// The months of the master games `list` names that `sums` gives a digest for, oldest first,
/// and how many it names without one.
pub fn months(list: &str, sums: &str) -> (Vec<Month>, usize) {
    archive_months(list, sums, PREFIX)
}

/// The months `list` names (files of `prefix`) that `sums` gives a digest for, oldest first, and
/// how many it names without one. Only a file's name is taken from `list`: every month is fetched
/// from the archive itself, never from a link the list gives.
pub(super) fn archive_months(list: &str, sums: &str, prefix: &str) -> (Vec<Month>, usize) {
    let digests: HashMap<&str, &str> = sums
        .lines()
        .filter_map(|line| {
            let (digest, file) = line.trim().split_once(char::is_whitespace)?;
            // `sha256sum` marks a file it read as binary with `*`.
            let file = file.trim_start().trim_start_matches('*');
            let valid = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
            valid.then_some((file, digest))
        })
        .collect();
    let mut months = BTreeMap::new();
    let mut unchecked = HashSet::new();
    for line in list.lines() {
        let file = line.trim().rsplit('/').next().unwrap_or_default();
        let Some(month) = month_of(file, prefix) else {
            continue;
        };
        let Some(digest) = digests.get(file) else {
            unchecked.insert(file);
            continue;
        };
        months.insert(
            month.to_string(),
            Month {
                month: month.to_string(),
                file: file.to_string(),
                sha256: digest.to_ascii_lowercase(),
            },
        );
    }
    (months.into_values().collect(), unchecked.len())
}

/// The archive files the database holds: the file each finished import read.
pub(super) fn installed(db: &Connection) -> Result<HashSet<String>> {
    let mut statement = db
        .prepare("SELECT file FROM imports WHERE finished_at IS NOT NULL AND file IS NOT NULL")
        .sql()?;
    let files = statement
        .query_map([], |row| row.get::<_, String>(0))
        .sql()?
        .collect::<rusqlite::Result<HashSet<String>>>()
        .sql()?;
    Ok(files)
}

/// How the master games download words its failures.
const WORDING: download::Wording = download::Wording {
    network: |error| {
        if error.is_timeout() {
            "The master games download timed out.".to_string()
        } else {
            format!("Could not download the master games: {error}")
        }
    },
    status: |status| format!("Lichess answered {status} for the master games."),
    too_large: "The master games download is unexpectedly large.",
};

/// A download saved to a file, removed however its import ends.
pub(super) struct Partial(pub PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        // Already gone when the download never started; `sweep` removes what is left over.
        std::fs::remove_file(&self.0).ok();
    }
}

/// One of an archive's index files (`list.txt`, `sha256sums.txt`), as text.
pub(super) async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
    cancel: &CancellationToken,
    usage: download::Usage<'_>,
    wording: &download::Wording,
) -> Result<String> {
    let limits = download::Limits {
        max_bytes: MAX_INDEX,
        deadline: Some(Instant::now() + INDEX_TIMEOUT),
    };
    let body = download::open(client, || client.get(url), limits, cancel, usage, wording)
        .await?
        .bytes()
        .await?;
    String::from_utf8(body)
        .map_err(|_| CoreError::new("Lichess's list of games could not be read."))
}

/// Downloads `month` from `url` into `path`, reporting its bytes, and checks its digest.
pub(super) async fn fetch_file(
    client: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
    expected: &Expected<'_>,
    cancel: &CancellationToken,
    usage: download::Usage<'_>,
    reporter: &mut Reporter,
) -> Result<()> {
    let staging = |cause| CoreError::wrap("Could not save the download", cause);
    let limits = download::Limits {
        max_bytes: expected.max_bytes,
        deadline: None,
    };
    let mut body = download::open(
        client,
        || client.get(url),
        limits,
        cancel,
        usage,
        expected.wording,
    )
    .await?;
    let mut count = import::Count {
        total_bytes: body.total().unwrap_or(0),
        ..import::Count::default()
    };
    reporter.send(Phase::Downloading, count, true);
    let mut file = tokio::fs::File::create(path).await.map_err(staging)?;
    let mut hasher = Sha256::new();
    let received: Result<()> = async {
        while let Some(chunk) = body.next().await? {
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(staging)?;
            count.bytes = body.received();
            reporter.send(Phase::Downloading, count, false);
        }
        Ok(())
    }
    .await;
    // However the download ended, its last write lands before the file closes, so a failed or
    // cancelled month's file can be removed at once (Windows keeps a file open elsewhere).
    let flushed = file.flush().await.map_err(staging);
    drop(file);
    received?;
    flushed?;
    if kchess_delta::hex(&hasher.finalize()) != expected.sha256 {
        return Err(CoreError::new(expected.broken.clone()));
    }
    Ok(())
}

/// What a download saved to a file (`fetch_file`) must be: its SHA-256 (lowercase hex), at most
/// `max_bytes`, its failures in `wording`'s words and a digest other than its own in `broken`.
pub(super) struct Expected<'a> {
    pub sha256: &'a str,
    pub max_bytes: u64,
    pub wording: &'a download::Wording,
    pub broken: String,
}

impl GameDbService {
    /// Brings the months of the master games the database lacks from the archive at `archive`
    /// (`ARCHIVE`, or a test's), the newest `keep` months of it when given. Progress arrives as
    /// `gamedb:progress`; `cancel` stops it, keeping the months already in.
    pub async fn master_install(
        &self,
        archive: &str,
        keep: Option<u32>,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let result = self.master_install_inner(archive, keep, meter).await;
        self.inner.logged("master games download", result)
    }

    async fn master_install_inner(
        &self,
        archive: &str,
        keep: Option<u32>,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let mut guard = self.claim(ID)?;
        let running = Arc::clone(&guard.running);
        let count = match self.bring_months(archive, keep, meter, &running).await {
            Ok(count) => count,
            Err(error) => {
                guard.end = Some(failed(&running, ID, &error));
                return Err(error);
            }
        };
        let database =
            self.inner.info(ID).await?.ok_or_else(|| {
                CoreError::new("The database could not be read after the download.")
            })?;
        guard.end = Some(progress(Phase::Done, ID, count, None));
        drop(guard);
        Ok(GameDbImported {
            database,
            added: count.games,
            skipped: count.skipped,
        })
    }

    /// Downloads and reads in each missing month, oldest first: what they added.
    async fn bring_months(
        &self,
        archive: &str,
        keep: Option<u32>,
        meter: &dyn Meter,
        running: &Running,
    ) -> Result<import::Count> {
        let inner = &self.inner;
        let cancel = &running.cancel;
        let client = download::client(Some(SILENCE))?;
        let usage = download::Usage {
            meter,
            kind: UsageKind::Database,
        };
        let list = format!("{archive}list.txt");
        let list = fetch_text(&client, &list, cancel, usage, &WORDING).await?;
        let sums = format!("{archive}sha256sums.txt");
        let sums = fetch_text(&client, &sums, cancel, usage, &WORDING).await?;
        let (mut months, unchecked) = months(&list, &sums);
        if unchecked > 0 {
            inner.host.log(
                Level::Warn,
                "gamedb",
                &format!("{unchecked} month(s) of master games have no checksum yet; left out"),
            );
        }
        if let Some(keep) = keep {
            let older = months.len().saturating_sub(keep as usize);
            months.drain(..older);
        }
        if months.is_empty() {
            return Err(CoreError::new("Lichess lists no master games to download."));
        }
        let file = inner.file(ID);
        let held = if file.is_file() {
            inner.reader(ID)?.run(installed).await?
        } else {
            HashSet::new()
        };
        std::fs::create_dir_all(&inner.dir)
            .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
        let mut total = import::Count::default();
        for month in months.into_iter().filter(|m| !held.contains(&m.file)) {
            let partial = Partial(
                inner
                    .dir
                    .join(format!("{ID}-{}{SUFFIX}.partial", month.month)),
            );
            let mut reporter = Reporter::new(&inner.host, ID, cancel);
            reporter.earlier = total;
            reporter.month = Some(month.month.clone());
            let url = format!("{archive}{}", month.file);
            let expected = Expected {
                sha256: &month.sha256,
                max_bytes: MAX_MONTH,
                wording: &WORDING,
                broken: format!(
                    "The master games of {} did not arrive intact. Try again.",
                    month.month
                ),
            };
            fetch_file(
                &client,
                &url,
                &partial.0,
                &expected,
                cancel,
                usage,
                &mut reporter,
            )
            .await?;
            let target = if file.is_file() {
                import::Target::Existing { file: file.clone() }
            } else {
                inner.new_target(
                    ID,
                    NAME.to_string(),
                    DatabaseSource::Master,
                    Some(ATTRIBUTION.to_string()),
                )
            };
            reporter.send(Phase::Reading, import::Count::default(), true);
            let (path, name) = (partial.0.clone(), month.file.clone());
            let count = inner
                .run_import(target, PgnPacking::Zstd, reporter, move || {
                    import::Source::downloaded(&path, &name)
                })
                .await?;
            drop(partial);
            total.games += count.games;
            total.skipped += count.skipped;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archive_lists_its_months_by_name_with_their_digests() {
        let digest = |c: char| c.to_string().repeat(64);
        let list = "https://database.lichess.org/broadcast/lichess_db_broadcast_2026-10.pgn.zst\n\
            https://elsewhere.example/lichess_db_broadcast_2026-09.pgn.zst\n\
            lichess_db_broadcast_2026-08.pgn.zst\n\
            https://database.lichess.org/broadcast/lichess_db_broadcast_2026-13.pgn.zst\n\
            https://database.lichess.org/broadcast/notes.txt\n\
            https://database.lichess.org/broadcast/lichess_db_broadcast_2026-07.pgn.zst";
        let sums = format!(
            "{}  lichess_db_broadcast_2026-10.pgn.zst\n{} *lichess_db_broadcast_2026-09.pgn.zst\n\
             {}  lichess_db_broadcast_2026-08.pgn.zst\nnot-a-digest  lichess_db_broadcast_2026-07.pgn.zst\n",
            digest('a'),
            digest('B'),
            digest('c'),
        );
        let (months, unchecked) = months(list, &sums);
        let named: Vec<(&str, &str)> = months
            .iter()
            .map(|m| (m.month.as_str(), m.file.as_str()))
            .collect();
        assert_eq!(
            named,
            [
                ("2026-08", "lichess_db_broadcast_2026-08.pgn.zst"),
                ("2026-09", "lichess_db_broadcast_2026-09.pgn.zst"),
                ("2026-10", "lichess_db_broadcast_2026-10.pgn.zst"),
            ]
        );
        assert_eq!(months[1].sha256, digest('b'));
        // 2026-07's digest is not one; 2026-13 is no month.
        assert_eq!(unchecked, 1);
    }
}
