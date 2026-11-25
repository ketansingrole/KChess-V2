//! The offline endgame tablebases: the Syzygy tables of every position with up to five pieces,
//! downloaded once into the profile's `tablebases` folder (`tablebaseInstall`) and probed there,
//! so a tablebase lookup of such a position needs no connection (`positionLookup`).
//!
//! The files come from Lichess's mirror of the tables. `files.txt` pins each one's name, size and
//! SHA-256 as the mirror lists them (`sha256`, `bytes.tsv`), so a file is kept only when it
//! matches: it is written as `<name>.part`, checked, then renamed. A file of the right size on
//! disk was checked when it arrived, so a download stopped part way goes on from the files it
//! has, and the tables already there answer meanwhile.
//!
//! A probe answers as Lichess's tablebase does (`lichess::lookups`): the position's outcome for
//! the side to move, and each move's for the side to move after it, the best first. A position
//! the tables on disk do not cover (more pieces, castling rights, or a table not yet downloaded)
//! is asked of Lichess instead.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use kchess_contracts::types::{
    LookupMove, PositionLookup, PositionLookupKind, SavedLookup, TablebasePhase, TablebaseProgress,
    TablebaseStatus, UsageKind,
};
use sha2::{Digest as _, Sha256};
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::{CastlingMode, Chess, Position as _};
use shakmaty_syzygy::{AmbiguousWdl, SyzygyError, Tablebase};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::download;
use crate::host::{Host, Level};
use crate::tasks::lock;
use crate::usage::Meter;
use kchess_contracts::error::{CoreError, Result};

/// Every table of up to five pieces: `<name> <bytes> <sha256>` a line.
pub const FILES: &str = include_str!("files.txt");
/// Lichess's mirror of the tables, each under `3-4-5-wdl/` or `3-4-5-dtz/`.
pub const ORIGIN: &str = "https://tablebase.lichess.ovh/tables/standard";
/// The most pieces the tables cover.
pub const PIECES: u32 = 5;
/// The files downloaded at once.
const PARALLEL: usize = 4;
/// A server that sends nothing for this long ends the download.
const SILENCE: Duration = Duration::from_secs(60);
/// Progress is reported each time this many more bytes have arrived.
const REPORT_EVERY: u64 = 2 * 1024 * 1024;

const RUNNING: &str = "The tablebases are already downloading.";
const DOWNLOADING: &str = "The tablebases are still downloading.";
const CLOSING: &str = "KChess is closing.";
const MISMATCH: &str = "A tablebase file did not download correctly. Try again.";

/// One table file as the mirror publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

impl TableFile {
    /// Its address under `base`: win/draw/loss tables and distance-to-zero tables have folders
    /// of their own.
    fn url(&self, base: &str) -> String {
        let folder = if self.name.ends_with(".rtbz") {
            "3-4-5-dtz"
        } else {
            "3-4-5-wdl"
        };
        format!("{}/{folder}/{}", base.trim_end_matches('/'), self.name)
    }
}

/// The files `text` lists (`files.txt`'s lines). A line that is not a plain file name, a size and
/// a SHA-256 is refused, so no listed name can reach outside the folder.
pub fn parse_files(text: &str) -> Result<Vec<TableFile>> {
    let refused = |line: &str| CoreError::new(format!("Unreadable tablebase list line: {line}"));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split_whitespace();
            let (Some(name), Some(bytes), Some(sha256), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(refused(line));
            };
            let plain = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
                && (name.ends_with(".rtbw") || name.ends_with(".rtbz"));
            let hex = sha256.len() == 64 && sha256.chars().all(|c| c.is_ascii_hexdigit());
            match (plain, hex, bytes.parse::<u64>()) {
                (true, true, Ok(bytes)) => Ok(TableFile {
                    name: name.to_string(),
                    bytes,
                    sha256: sha256.to_ascii_lowercase(),
                }),
                _ => Err(refused(line)),
            }
        })
        .collect()
}

/// The tablebases' folder in the profile whose data directory is `data_dir`.
pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("tablebases")
}

#[derive(Default)]
struct State {
    /// The download running, which `cancel` stops.
    running: Option<CancellationToken>,
    /// The tables opened for probing, opened again after a download or a delete.
    tables: Option<Arc<Tablebase<Chess>>>,
}

struct Inner {
    host: Arc<dyn Host>,
    meter: Arc<dyn Meter>,
    directory: PathBuf,
    base: String,
    files: Vec<TableFile>,
    state: Mutex<State>,
    /// Held by the download running, so a second waits for none and closing waits for it.
    downloading: tokio::sync::Mutex<()>,
    /// Cancelled when the core closes: a download stops and no new one starts.
    closing: CancellationToken,
}

/// The profile's tablebases: their download, their status and their probes.
#[derive(Clone)]
pub struct Tablebases {
    inner: Arc<Inner>,
}

impl Tablebases {
    /// The tables `files` lists, downloaded from `base` into `directory`.
    pub fn new(
        host: Arc<dyn Host>,
        meter: Arc<dyn Meter>,
        directory: PathBuf,
        base: String,
        files: Vec<TableFile>,
    ) -> Tablebases {
        Tablebases {
            inner: Arc::new(Inner {
                host,
                meter,
                directory,
                base,
                files,
                state: Mutex::new(State::default()),
                downloading: tokio::sync::Mutex::new(()),
                closing: CancellationToken::new(),
            }),
        }
    }

    /// Which listed files are on disk at their size.
    async fn present(&self) -> Vec<bool> {
        let (directory, files) = (self.inner.directory.clone(), self.inner.files.clone());
        tokio::task::spawn_blocking(move || {
            files
                .iter()
                .map(|file| {
                    std::fs::metadata(directory.join(&file.name))
                        .is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// What is on disk, and whether a download runs.
    pub async fn status(&self) -> TablebaseStatus {
        let present = self.present().await;
        let files = &self.inner.files;
        let kept = || files.iter().zip(&present).filter(|(_, here)| **here);
        TablebaseStatus {
            installed: !files.is_empty() && present.iter().all(|here| *here),
            files: kept().count() as u32,
            total_files: files.len() as u32,
            bytes: kept().map(|(file, _)| file.bytes).sum(),
            total_bytes: files.iter().map(|file| file.bytes).sum(),
            pieces: PIECES,
            busy: lock(&self.inner.state).running.is_some(),
        }
    }

    /// Downloads the files not yet on disk, telling `report` how far it has come, and ends with
    /// the status. A second download while one runs is refused; `cancel` stops it, keeping the
    /// files it finished.
    pub async fn install(
        &self,
        report: impl Fn(TablebaseProgress) + Send + Sync + 'static,
    ) -> Result<TablebaseStatus> {
        let Ok(_held) = self.inner.downloading.try_lock() else {
            return Err(CoreError::new(RUNNING));
        };
        let token = {
            let mut state = lock(&self.inner.state);
            if self.inner.closing.is_cancelled() {
                return Err(CoreError::new(CLOSING));
            }
            let token = self.inner.closing.child_token();
            state.running = Some(token.clone());
            token
        };
        let _ended = crate::tasks::on_drop(|| {
            let mut state = lock(&self.inner.state);
            state.running = None;
            // Opened again with the new tables at the next probe.
            state.tables = None;
        });
        let report = Arc::new(report);
        let outcome = self.download(&token, Arc::clone(&report)).await;
        // A stopped or failed download drops the fetches still running before their own
        // cleanup: what they were writing goes here.
        self.remove_parts().await;
        let status = TablebaseStatus {
            // This download is over, though its guard lets go only as it returns.
            busy: false,
            ..self.status().await
        };
        let progress = |phase, message| TablebaseProgress {
            phase,
            received: status.bytes,
            total: status.total_bytes,
            message,
        };
        match outcome {
            Ok(()) => {
                report(progress(TablebasePhase::Done, None));
                Ok(status)
            }
            Err(_) if token.is_cancelled() => {
                report(progress(TablebasePhase::Cancelled, None));
                Ok(status)
            }
            Err(cause) => {
                self.inner.host.log(
                    Level::Warn,
                    "tablebase",
                    &format!("The tablebase download failed: {}", cause.message),
                );
                report(progress(
                    TablebasePhase::Failed,
                    Some(cause.message.clone()),
                ));
                Err(cause)
            }
        }
    }

    /// Stops the download running, if one is.
    pub fn cancel(&self) {
        if let Some(token) = &lock(&self.inner.state).running {
            token.cancel();
        }
    }

    /// Stops a download and waits for it to end, and starts no other: the core calls it before
    /// releasing the profile.
    pub async fn shutdown(&self) {
        self.inner.closing.cancel();
        // The download ends at its next chunk, and lets go as it returns.
        let _ended = self.inner.downloading.lock().await;
    }

    /// Removes every table, refused while they download.
    pub async fn delete(&self) -> Result<TablebaseStatus> {
        {
            let mut state = lock(&self.inner.state);
            if state.running.is_some() {
                return Err(CoreError::new(DOWNLOADING));
            }
            state.tables = None;
        }
        match tokio::fs::remove_dir_all(&self.inner.directory).await {
            Ok(()) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => return Err(CoreError::wrap("Could not delete the tablebases", cause)),
        }
        Ok(self.status().await)
    }

    /// Removes the `.part` files of downloads that did not finish.
    async fn remove_parts(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.inner.directory).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name().to_string_lossy().ends_with(".part")
                && let Err(cause) = tokio::fs::remove_file(entry.path()).await
            {
                self.inner.host.log(
                    Level::Warn,
                    "tablebase",
                    &format!("Could not remove an unfinished tablebase file: {cause}"),
                );
            }
        }
    }

    async fn download(
        &self,
        token: &CancellationToken,
        report: Arc<impl Fn(TablebaseProgress) + Send + Sync + 'static>,
    ) -> Result<()> {
        let directory = &self.inner.directory;
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|cause| CoreError::wrap("Could not create the tablebases folder", cause))?;
        let present = self.present().await;
        let total: u64 = self.inner.files.iter().map(|file| file.bytes).sum();
        let mut received = 0;
        let mut missing = Vec::new();
        for (file, here) in self.inner.files.iter().zip(present) {
            if here {
                received += file.bytes;
            } else {
                missing.push(file.clone());
            }
        }
        let received = AtomicU64::new(received);
        let reported = AtomicU64::new(0);
        let tell = |more: u64| {
            let now = received.fetch_add(more, Ordering::Relaxed) + more;
            let step = now / REPORT_EVERY;
            if reported.fetch_max(step, Ordering::Relaxed) < step {
                report(TablebaseProgress {
                    phase: TablebasePhase::Downloading,
                    received: now,
                    total,
                    message: None,
                });
            }
        };
        report(TablebaseProgress {
            phase: TablebasePhase::Downloading,
            received: received.load(Ordering::Relaxed),
            total,
            message: None,
        });
        let client = download::client(Some(SILENCE))?;
        let mut fetches = futures_util::stream::iter(missing)
            .map(|file| self.fetch(&client, file, token, &tell))
            .buffer_unordered(PARALLEL);
        // One file failing lets the others finish, so the next download fetches only it; a
        // cancelled download ends each fetch at its next chunk.
        let mut first = None;
        while let Some(fetched) = fetches.next().await {
            if let Err(cause) = fetched {
                first.get_or_insert(cause);
            }
        }
        first.map_or(Ok(()), Err)
    }

    /// Downloads `file`, keeps it only when it matches its size and SHA-256.
    async fn fetch(
        &self,
        client: &reqwest::Client,
        file: TableFile,
        token: &CancellationToken,
        tell: &(impl Fn(u64) + Sync),
    ) -> Result<()> {
        let path = self.inner.directory.join(&file.name);
        let part = self.inner.directory.join(format!("{}.part", file.name));
        let outcome = self.write(client, &file, &part, token, tell).await;
        // A file that failed stays a `.part`, which `install` removes.
        outcome?;
        tokio::fs::rename(&part, &path)
            .await
            .map_err(|cause| CoreError::wrap("Could not keep a tablebase file", cause))
    }

    async fn write(
        &self,
        client: &reqwest::Client,
        file: &TableFile,
        part: &Path,
        token: &CancellationToken,
        tell: &(impl Fn(u64) + Sync),
    ) -> Result<()> {
        let url = file.url(&self.inner.base);
        let usage = download::Usage {
            meter: &*self.inner.meter,
            kind: UsageKind::Analysis,
        };
        let limits = download::Limits {
            max_bytes: file.bytes,
            deadline: None,
        };
        let mut body =
            download::open(client, || client.get(&url), limits, token, usage, &WORDING).await?;
        let mut out = tokio::fs::File::create(part)
            .await
            .map_err(|cause| CoreError::wrap("Could not save a tablebase file", cause))?;
        let mut hash = Sha256::new();
        while let Some(chunk) = body.next().await? {
            hash.update(&chunk);
            out.write_all(&chunk)
                .await
                .map_err(|cause| CoreError::wrap("Could not save a tablebase file", cause))?;
            tell(chunk.len() as u64);
        }
        out.flush()
            .await
            .map_err(|cause| CoreError::wrap("Could not save a tablebase file", cause))?;
        if body.received() != file.bytes || kchess_delta::hex(&hash.finalize()) != file.sha256 {
            return Err(CoreError::new(MISMATCH));
        }
        Ok(())
    }

    /// The tables on disk, opened once.
    fn tables(&self) -> Option<Arc<Tablebase<Chess>>> {
        let mut state = lock(&self.inner.state);
        if let Some(tables) = &state.tables {
            return Some(Arc::clone(tables));
        }
        let mut tables = Tablebase::new();
        // A missing folder is no tables yet.
        let added = tables.add_directory(&self.inner.directory).unwrap_or(0);
        if added == 0 {
            return None;
        }
        let tables = Arc::new(tables);
        state.tables = Some(Arc::clone(&tables));
        Some(tables)
    }

    /// `fen` looked up in the tables on disk, as `positionLookup`'s tablebase answers it; `None`
    /// when they do not cover it, so Lichess's tablebase is asked instead.
    pub async fn lookup(&self, fen: &str, now: i64) -> Option<PositionLookup> {
        let pos: Chess = fen
            .parse::<Fen>()
            .ok()?
            .into_position(CastlingMode::Standard)
            .ok()?;
        if pos.board().occupied().count() > PIECES as usize || pos.castles().any() {
            return None;
        }
        let fen = fen.to_string();
        let this = self.clone();
        let saved = tokio::task::spawn_blocking(move || {
            let tables = this.tables()?;
            match probe(&tables, &pos, &fen, now) {
                Ok(saved) => Some(saved),
                Err(SyzygyError::MissingTable { .. }) => None,
                Err(cause) => {
                    this.inner.host.log(
                        Level::Warn,
                        "tablebase",
                        &format!("A tablebase probe failed: {cause}"),
                    );
                    None
                }
            }
        })
        .await
        .ok()??;
        Some(PositionLookup {
            saved,
            cached: false,
            stale: false,
            message: None,
        })
    }
}

/// How the downloads word their failures.
const WORDING: download::Wording = download::Wording {
    network: |cause| {
        if cause.is_timeout() {
            "The tablebase download timed out.".to_string()
        } else {
            "The tablebase download failed (network error).".to_string()
        }
    },
    status: |status| format!("The tablebase download failed: HTTP {status}."),
    too_large: MISMATCH,
};

/// The outcome's name as Lichess's tablebase gives it.
fn category(wdl: AmbiguousWdl) -> &'static str {
    match wdl {
        AmbiguousWdl::Win => "win",
        AmbiguousWdl::MaybeWin => "maybe-win",
        AmbiguousWdl::CursedWin => "cursed-win",
        AmbiguousWdl::Draw => "draw",
        AmbiguousWdl::BlessedLoss => "blessed-loss",
        AmbiguousWdl::MaybeLoss => "maybe-loss",
        AmbiguousWdl::Loss => "loss",
    }
}

/// The outcome for the side to move in `pos`, and its distance to zeroing.
fn judged(
    tables: &Tablebase<Chess>,
    pos: &Chess,
) -> std::result::Result<(AmbiguousWdl, i64), SyzygyError> {
    if pos.is_checkmate() {
        return Ok((AmbiguousWdl::Loss, 0));
    }
    if pos.is_stalemate() || pos.is_insufficient_material() {
        return Ok((AmbiguousWdl::Draw, 0));
    }
    let dtz = tables.probe_dtz(pos)?;
    let wdl = AmbiguousWdl::from_dtz_and_halfmoves(dtz, pos.halfmoves());
    Ok((wdl, i64::from(dtz.ignore_rounding().0)))
}

/// Where a move's outcome, for the side to move after it, ranks: the opponent's loss first.
fn rank(wdl: AmbiguousWdl) -> u8 {
    match wdl {
        AmbiguousWdl::Loss => 0,
        AmbiguousWdl::MaybeLoss => 1,
        AmbiguousWdl::BlessedLoss => 2,
        AmbiguousWdl::Draw => 3,
        AmbiguousWdl::CursedWin => 4,
        AmbiguousWdl::MaybeWin => 5,
        AmbiguousWdl::Win => 6,
    }
}

/// The lookup of `pos` (written `fen`) in `tables`: its outcome, and every move's, the best
/// first: a mate, then a win that resets the fifty-move count, then the fastest win; the
/// longest resistance in a loss.
fn probe(
    tables: &Tablebase<Chess>,
    pos: &Chess,
    fen: &str,
    now: i64,
) -> std::result::Result<SavedLookup, SyzygyError> {
    let (wdl, dtz) = judged(tables, pos)?;
    let mut moves = Vec::new();
    for m in pos.legal_moves() {
        let mut after = pos.clone();
        let san = SanPlus::from_move_and_play_unchecked(&mut after, m);
        let (theirs, their_dtz) = judged(tables, &after)?;
        let key = (
            rank(theirs),
            !after.is_checkmate(),
            !(theirs.signum() < 0 && m.is_zeroing()),
            std::cmp::Reverse(their_dtz),
            san.to_string(),
        );
        moves.push((
            key,
            LookupMove {
                uci: m.to_uci(CastlingMode::Standard).to_string(),
                san: san.to_string(),
                category: Some(category(theirs).to_string()),
                dtz: Some(Some(their_dtz)),
                ..LookupMove::default()
            },
        ));
    }
    moves.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(SavedLookup {
        kind: PositionLookupKind::Tablebase,
        fen: fen.to_string(),
        fetched_at: now,
        total: None,
        opening: None,
        category: Some(category(wdl).to_string()),
        dtz: Some(Some(dtz)),
        games: None,
        moves: moves.into_iter().map(|(_, entry)| entry).collect(),
    })
}

#[cfg(test)]
mod tests;
