//! Engine games: the Top Chess Engine Championship's games (`github.com/TCEC-Chess/tcecgames`,
//! CC BY-SA 3.0), one database (`tcec`) of every standard-chess competition game TCEC has played.
//!
//! TCEC publishes a release a season, each holding every game so far, and GitHub gives each
//! release file its SHA-256. The latest release (`releases/latest` of GitHub's API) names its tag;
//! the compact zip (`TCEC-everything-compact.zip`, some 27 MB) is fetched from that tag, never
//! from a link the answer gives, checked against its digest, and only its standard competitions'
//! file is read (47 MB of PGN in 2026): the zip also holds the Chess960 and Double Chess960
//! competitions and the bonus and test games. A newer release replaces the database's games
//! (`import::Target::Replace`); the release the database holds is the file its import recorded,
//! so the same release is never downloaded twice.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::master::{Expected, Partial, SILENCE, fetch_file, installed};
use super::{GameDbService, Reporter, Running, failed, import, progress};
use crate::download;
use crate::usage::Meter;
use kchess_contracts::check::PgnPacking;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{DatabaseSource, GameDbImported, GameDbPhase as Phase, UsageKind};

/// The engine games database's id.
pub const ID: &str = "tcec";
/// Its name as first listed; the person may rename it.
pub const NAME: &str = "TCEC engine games";
/// Whose games they are and on what terms, as the licence asks to be shown.
pub const ATTRIBUTION: &str = "TCEC games, tcec-chess.com, CC BY-SA 3.0";
/// GitHub's answer naming the latest release.
pub const LATEST: &str = crate::sources::TCEC.url;
/// Where a release's files are, by tag: `{DOWNLOADS}{tag}/{ASSET}`.
pub const DOWNLOADS: &str = crate::sources::TCEC_DOWNLOADS;
/// The release file KChess reads: every game, its comments replaced by book exits.
pub const ASSET: &str = crate::sources::TCEC_ASSET;
/// The file of it KChess keeps: the standard-chess competitions.
const ENTRY: &str = "TCEC-everything-compet-traditional.pgn";
/// The most the zip may bring: 27 MB in 2026, growing some 3 MB a season.
const MAX_ASSET: u64 = 512 * 1024 * 1024;
/// The most GitHub's answer may bring, and how long it may take.
const MAX_RELEASE: u64 = 1024 * 1024;
const RELEASE_TIMEOUT: Duration = Duration::from_secs(60);

/// How the engine games download words its failures.
const WORDING: download::Wording = download::Wording {
    network: |error| {
        if error.is_timeout() {
            "The engine games download timed out.".to_string()
        } else {
            format!("Could not download the engine games: {error}")
        }
    },
    status: |status| format!("GitHub answered {status} for the engine games."),
    too_large: "The engine games download is unexpectedly large.",
};

/// The part of GitHub's answer about a release that KChess reads.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    /// `sha256:` and the file's digest; absent from releases before GitHub kept them (2025).
    #[serde(default)]
    digest: Option<String>,
}

/// The release to download: its tag and the zip's digest.
#[derive(Debug, PartialEq, Eq)]
struct Latest {
    tag: String,
    sha256: String,
}

/// The release GitHub's answer `text` names, when it has the zip with a digest: its tag is a
/// name only (a URL is built from it), and its digest 64 hex digits.
fn latest(text: &str) -> Result<Latest> {
    let release: Release = serde_json::from_str(text)
        .map_err(|_| CoreError::new("GitHub's list of engine games could not be read."))?;
    let tag = release.tag_name;
    let named = !tag.is_empty()
        && tag.len() <= 64
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !tag.starts_with('.');
    if !named {
        return Err(CoreError::new(
            "GitHub's list of engine games could not be read.",
        ));
    }
    let sha256 = release
        .assets
        .iter()
        .find(|asset| asset.name == ASSET)
        .and_then(|asset| asset.digest.as_deref()?.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            CoreError::new("TCEC's latest games are not ready to download yet. Try again later.")
        })?;
    Ok(Latest { tag, sha256 })
}

/// What the import records of a release: its tag and file.
fn label(tag: &str) -> String {
    format!("{tag}/{ASSET}")
}

/// Whether a file of the zip is the one KChess reads.
fn entry(name: &str) -> bool {
    name.rsplit(['/', '\\']).next() == Some(ENTRY)
}

impl GameDbService {
    /// Brings TCEC's latest release (`latest`: `LATEST`, or a test's; its files under
    /// `downloads`) into the engine games database, unless it holds that release already. Progress
    /// arrives as `gamedb:progress`; `cancel` stops it and keeps the database as it was.
    pub async fn tcec_install(
        &self,
        latest: &str,
        downloads: &str,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let result = self.tcec_install_inner(latest, downloads, meter).await;
        self.inner.logged("engine games download", result)
    }

    async fn tcec_install_inner(
        &self,
        latest: &str,
        downloads: &str,
        meter: &dyn Meter,
    ) -> Result<GameDbImported> {
        let mut guard = self.claim(ID)?;
        let running = Arc::clone(&guard.running);
        let count = match self.bring_release(latest, downloads, meter, &running).await {
            Ok(count) => count,
            Err(error) => {
                let error = if running.cancel.is_cancelled() {
                    CoreError::new("Download cancelled.")
                } else {
                    error
                };
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

    /// Downloads and reads in the latest release, when the database does not hold it: what it
    /// added (nothing when it does).
    async fn bring_release(
        &self,
        latest_url: &str,
        downloads: &str,
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
        let mut reporter = Reporter::new(&inner.host, ID, cancel);
        reporter.send(Phase::Downloading, import::Count::default(), true);
        let text = release_text(&client, latest_url, cancel, usage).await?;
        let release = latest(&text)?;
        let file = inner.file(ID);
        let label = label(&release.tag);
        if file.is_file() && inner.reader(ID)?.run(installed).await?.contains(&label) {
            return Ok(import::Count::default());
        }
        std::fs::create_dir_all(&inner.dir)
            .map_err(|e| CoreError::wrap("Could not create the databases folder", e))?;
        let partial = Partial(inner.dir.join(format!("{ID}-{}.zip.partial", release.tag)));
        let expected = Expected {
            sha256: &release.sha256,
            max_bytes: MAX_ASSET,
            wording: &WORDING,
            broken: "The engine games did not arrive intact. Try again.".to_string(),
        };
        let url = format!("{downloads}{}/{ASSET}", release.tag);
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
            import::Target::Replace { file: file.clone() }
        } else {
            inner.new_target(
                ID,
                NAME.to_string(),
                DatabaseSource::Tcec,
                Some(ATTRIBUTION.to_string()),
            )
        };
        reporter.send(Phase::Reading, import::Count::default(), true);
        let path = partial.0.clone();
        let count = inner
            .run_import(target, PgnPacking::Zip, reporter, move || {
                Ok(import::Source::downloaded(&path, &label)?.only(entry))
            })
            .await?;
        drop(partial);
        // The last position search's games may be this database's old ones.
        inner.listed.forget();
        Ok(count)
    }
}

/// GitHub's answer about the latest release, as text.
async fn release_text(
    client: &reqwest::Client,
    url: &str,
    cancel: &CancellationToken,
    usage: download::Usage<'_>,
) -> Result<String> {
    let limits = download::Limits {
        max_bytes: MAX_RELEASE,
        deadline: Some(Instant::now() + RELEASE_TIMEOUT),
    };
    let request = || {
        client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
    };
    let body = download::open(client, request, limits, cancel, usage, &WORDING)
        .await?
        .bytes()
        .await?;
    String::from_utf8(body)
        .map_err(|_| CoreError::new("GitHub's list of engine games could not be read."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(tag: &str, digest: Option<&str>) -> String {
        serde_json::json!({
            "tag_name": tag,
            "assets": [
                { "name": "MD5SUM", "digest": format!("sha256:{}", "1".repeat(64)) },
                { "name": ASSET, "digest": digest },
            ],
        })
        .to_string()
    }

    #[test]
    fn the_latest_release_names_its_tag_and_the_zips_digest() {
        let digest = format!("sha256:{}", "AB".repeat(32));
        assert_eq!(
            latest(&answer("S29-final", Some(&digest))).unwrap(),
            Latest {
                tag: "S29-final".into(),
                sha256: "ab".repeat(32),
            }
        );
        // No digest for the zip: nothing to check it against.
        let error = latest(&answer("S29-final", None)).unwrap_err();
        assert!(error.message.contains("not ready"), "{}", error.message);
        let short = format!("sha256:{}", "a".repeat(10));
        assert!(latest(&answer("S29-final", Some(&short))).is_err());
        // A tag is a name, never a path or a link.
        for tag in ["../x", "a/b", "", ".hidden", "https://example.com"] {
            assert!(latest(&answer(tag, Some(&digest))).is_err(), "{tag}");
        }
        assert!(latest("not json").is_err());
    }

    #[test]
    fn only_the_standard_competitions_file_is_read() {
        assert!(entry("TCEC-everything-compet-traditional.pgn"));
        assert!(entry(
            "compact/everything/TCEC-everything-compet-traditional.pgn"
        ));
        assert!(!entry("TCEC-everything.pgn"));
        assert!(!entry("TCEC-everything-compet-frc.pgn"));
        assert!(!entry("TCEC-everything-bonus-test.pgn"));
        assert_eq!(label("S29-final"), "S29-final/TCEC-everything-compact.zip");
    }
}
