//! `cargo xtask source-check`: whether every third-party download the core makes
//! (`crates/kchess-core/src/sources.rs`) still serves what KChess reads. Each source is asked only
//! for a small part: an archive's `list.txt` and `sha256sums.txt` and the first bytes of its newest
//! month, a GitHub release's listing, the first bytes of a file and its size, or one player's
//! latest game. A source that went silent, moved, or answers with something else (a parked or
//! taken-over domain) fails the run, naming the source, its owner and the address. Every request
//! has a deadline (`MAX_SECONDS`). The weekly Sources workflow (`.github/workflows/sources.yml`)
//! runs it; `ci.yml` never does, so an outage upstream cannot turn a pull request red.

use std::io::Write as _;
use std::process::{Command, Stdio};

use sha2::{Digest as _, Sha256};

use crate::pinned::shipped::SHIPPED;
use crate::util::{Result, civil_date, say, unix_now};

// The core's addresses, compiled here as well: plain data, so this checks the very addresses the
// core downloads from. The services are there for their owners; nothing here fetches them.
#[allow(dead_code)]
#[path = "../../../crates/kchess-core/src/sources.rs"]
mod sources;

use sources::Source;

/// The core's table files, `<name> <bytes> <sha256>` a line.
const TABLE_FILES: &str = include_str!("../../../crates/kchess-core/src/tablebase/files.txt");

/// The longest a request may take, connecting included.
const MAX_SECONDS: u32 = 60;
/// The longest a connection may take to open.
const CONNECT_SECONDS: u32 = 20;
/// The most an answer may bring: an index or a listing is a few hundred KB at most.
const MAX_ANSWER: u64 = 8 * 1024 * 1024;
/// How KChess's check names itself (Lichess refuses a request that names no program).
const USER_AGENT: &str = concat!(
    "KChess-source-check/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/ketansingrole/KChess-V2)"
);
/// The months an archive may go without a new one before it counts as silent: Lichess publishes
/// each month in the first days of the next.
const SILENT_MONTHS: i64 = 3;
/// Accounts whose games are always there to read.
const LICHESS_PLAYER: &str = "thibault";
const CHESSCOM_PLAYER: &str = "hikaru";
/// The size of the speech model KChess pins by SHA-256 (`crates/kchess-core/src/voice/mod.rs`).
const VOSK_MODEL_BYTES: u64 = 41_205_931;
/// The puzzle database has grown past this since 2023; far less is not the database.
const MIN_PUZZLE_BYTES: u64 = 100 * 1024 * 1024;
/// The table files fetched whole: the smallest, whose sizes and digests the core pins.
const TABLES: [&str; 2] = ["KQvK.rtbw", "KQvK.rtbz"];

/// A zstd frame's first bytes.
const ZSTD: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// A zstd skippable frame's, whose first byte is any of 0x50 to 0x5F: Lichess's files open with
/// one (the seek table a parallel compressor writes).
const ZSTD_SKIPPABLE: [u8; 4] = [0x50, 0x2A, 0x4D, 0x18];
const ZIP: [u8; 4] = *b"PK\x03\x04";

/// Whether `bytes` open a zstd file: a frame, or a skippable frame.
fn is_zstd(bytes: &[u8]) -> bool {
    let skippable =
        bytes.len() >= 4 && bytes[0] & 0xF0 == 0x50 && bytes[1..4] == ZSTD_SKIPPABLE[1..];
    bytes.starts_with(&ZSTD) || skippable
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&ZIP)
}

/// One request: the address, and the first bytes alone when `range` is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub range: Option<u64>,
    pub accept: Option<&'static str>,
}

impl Request {
    fn get(url: impl Into<String>) -> Request {
        Request {
            url: url.into(),
            range: None,
            accept: None,
        }
    }

    /// The first `bytes` of `url` and, from the answer's `Content-Range`, the whole file's size.
    fn head(url: impl Into<String>, bytes: u64) -> Request {
        Request {
            range: Some(bytes),
            ..Request::get(url)
        }
    }
}

/// A successful answer: its body and, for a range request, the whole file's size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub body: Vec<u8>,
    pub total: Option<u64>,
}

impl Answer {
    fn text(&self, url: &str) -> Result<&str> {
        std::str::from_utf8(&self.body).map_err(|_| format!("{url} answered with no text"))
    }
}

/// How a check asks: curl in a run, fixtures in a test.
pub type Fetch<'a> = dyn FnMut(&Request) -> Result<Answer> + 'a;

/// What a source's check found, for the report: a few words of evidence.
type Found = Result<String>;

/// What the checks compare against besides the answers: today's UTC month, and the table files
/// the core pins (`files.txt`'s lines).
#[derive(Clone, Copy)]
struct Known<'a> {
    today: (i64, u32),
    tables: &'a str,
}

type Check = fn(&mut Fetch, &Known) -> Found;

/// The check of `source`, when it is one of the downloads.
fn check_of(source: &Source) -> Option<Check> {
    let check: Check = match source.url {
        url if url == sources::LICHESS_BROADCAST.url => |fetch, known| {
            archive(
                fetch,
                &sources::LICHESS_BROADCAST,
                "lichess_db_broadcast_",
                known.today,
            )
        },
        url if url == sources::LICHESS_STANDARD.url => |fetch, known| {
            archive(
                fetch,
                &sources::LICHESS_STANDARD,
                "lichess_db_standard_rated_",
                known.today,
            )
        },
        url if url == sources::LICHESS_PUZZLES.url => |fetch, _| puzzles(fetch),
        url if url == sources::LICHESS.url => |fetch, _| lichess_games(fetch),
        url if url == sources::TCEC.url => |fetch, _| tcec(fetch),
        url if url == sources::STOCKFISH.url => |fetch, _| stockfish(fetch),
        url if url == sources::VOSK_MODEL.url => |fetch, _| vosk_model(fetch),
        url if url == sources::SYZYGY.url => |fetch, known| syzygy(fetch, known.tables),
        url if url == sources::CHESSCOM.url => |fetch, _| chesscom_games(fetch),
        _ => return None,
    };
    Some(check)
}

/// The months a Lichess archive's `list.txt` names (`YYYY-MM`), newest first: each line the
/// archive's address, `prefix`, the month and `.pgn.zst`.
fn listed_months(list: &str, archive: &str, prefix: &str) -> Vec<String> {
    let mut months: Vec<String> = list
        .lines()
        .filter_map(|line| month_of(line.trim().strip_prefix(archive)?, prefix))
        .map(str::to_string)
        .collect();
    months.sort_unstable_by(|a, b| b.cmp(a));
    months.dedup();
    months
}

/// The month `file` holds when it is `prefix`, `YYYY-MM` and `.pgn.zst`.
fn month_of<'a>(file: &'a str, prefix: &str) -> Option<&'a str> {
    let month = file.strip_prefix(prefix)?.strip_suffix(".pgn.zst")?;
    let (year, number) = month.split_once('-')?;
    let digits = |part: &str, len| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let number_ok = digits(number, 2) && (1..=12).contains(&number.parse::<u32>().ok()?);
    (digits(year, 4) && number_ok).then_some(month)
}

/// The files `sha256sums.txt` gives a digest for.
fn summed(sums: &str) -> Vec<&str> {
    sums.lines()
        .filter_map(|line| {
            let (digest, file) = line.trim().split_once(char::is_whitespace)?;
            let valid = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
            valid.then(|| file.trim_start().trim_start_matches('*'))
        })
        .collect()
}

/// How many months `month` (`YYYY-MM`) is before `today`.
fn months_before(month: &str, (year, number): (i64, u32)) -> Option<i64> {
    let (y, m) = month.split_once('-')?;
    let (y, m): (i64, i64) = (y.parse().ok()?, m.parse().ok()?);
    Some(year * 12 + i64::from(number) - (y * 12 + m))
}

fn magic(answer: &Answer, opens: fn(&[u8]) -> bool, what: &str, url: &str) -> Result {
    if opens(&answer.body) {
        Ok(())
    } else {
        Err(format!("{url} does not start as {what} does"))
    }
}

/// A Lichess archive: its list names recent months, each with a digest, and the newest is zstd.
fn archive(fetch: &mut Fetch, source: &Source, prefix: &str, today: (i64, u32)) -> Found {
    let list_url = format!("{}list.txt", source.url);
    let list = fetch(&Request::get(&list_url))?;
    let months = listed_months(list.text(&list_url)?, source.url, prefix);
    let newest = months
        .first()
        .ok_or_else(|| format!("{list_url} names no {prefix}YYYY-MM.pgn.zst month"))?;
    let behind = months_before(newest, today).unwrap_or(i64::MAX);
    if behind >= SILENT_MONTHS {
        return Err(format!(
            "{list_url} has had no new month since {newest}, {behind} months ago"
        ));
    }
    let sums_url = format!("{}sha256sums.txt", source.url);
    let sums = fetch(&Request::get(&sums_url))?;
    let file = format!("{prefix}{newest}.pgn.zst");
    if !summed(sums.text(&sums_url)?).contains(&file.as_str()) {
        return Err(format!("{sums_url} has no SHA-256 for {file}"));
    }
    let url = format!("{}{file}", source.url);
    magic(
        &fetch(&Request::head(&url, 4))?,
        is_zstd,
        "a zstd file",
        &url,
    )?;
    Ok(format!("newest {newest} of {} months", months.len()))
}

fn puzzles(fetch: &mut Fetch) -> Found {
    let url = sources::LICHESS_PUZZLES.url;
    let answer = fetch(&Request::head(url, 4))?;
    magic(&answer, is_zstd, "a zstd file", url)?;
    match answer.total {
        Some(total) if total >= MIN_PUZZLE_BYTES => Ok(format!("{} MB", total / 1_000_000)),
        total => Err(format!(
            "{url} is {} bytes, far less than the puzzle database",
            total.map_or("an unknown number of".into(), |total| total.to_string())
        )),
    }
}

/// One player's latest game, as the player games download reads them.
fn lichess_games(fetch: &mut Fetch) -> Found {
    let url = format!(
        "{}/api/games/user/{LICHESS_PLAYER}?max=1",
        sources::LICHESS.url
    );
    let answer = fetch(&Request {
        accept: Some("application/x-chess-pgn"),
        ..Request::get(&url)
    })?;
    let pgn = answer.text(&url)?;
    let site = format!("[Site \"{}/", sources::LICHESS.url);
    if pgn.contains("[Event \"") && pgn.contains(&site) {
        Ok(format!("{LICHESS_PLAYER}'s latest game"))
    } else {
        Err(format!("{url} answered with no Lichess game"))
    }
}

/// GitHub's answer about a release: its tag, and each asset's name, address and digest.
fn release(fetch: &mut Fetch, url: &str) -> Result<(String, Vec<serde_json::Value>)> {
    let answer = fetch(&Request::get(url))?;
    let release: serde_json::Value = serde_json::from_slice(&answer.body)
        .map_err(|_| format!("{url} answered with no GitHub release"))?;
    let tag = release["tag_name"]
        .as_str()
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| format!("{url} answered with no release tag"))?;
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    Ok((tag.to_string(), assets))
}

/// The asset `name` of a release, when it downloads from `downloads` under `tag`.
fn asset<'a>(
    assets: &'a [serde_json::Value],
    name: &str,
    downloads: &str,
    tag: &str,
) -> Option<&'a serde_json::Value> {
    let expected = format!("{downloads}{tag}/{name}");
    assets.iter().find(|asset| {
        asset["name"].as_str() == Some(name)
            && asset["browser_download_url"].as_str() == Some(expected.as_str())
    })
}

/// TCEC's latest release has the zip KChess reads, with a digest, and it is a zip.
fn tcec(fetch: &mut Fetch) -> Found {
    let (tag, assets) = release(fetch, sources::TCEC.url)?;
    let name = sources::TCEC_ASSET;
    let zip = asset(&assets, name, sources::TCEC_DOWNLOADS, &tag)
        .ok_or_else(|| format!("TCEC's release {tag} has no {name}"))?;
    let digest = zip["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"));
    if !digest.is_some_and(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(format!("{name} of TCEC's release {tag} has no SHA-256"));
    }
    let url = format!("{}{tag}/{name}", sources::TCEC_DOWNLOADS);
    magic(&fetch(&Request::head(&url, 4))?, is_zip, "a zip", &url)?;
    Ok(format!("release {tag}"))
}

/// The latest Stockfish release has every file KChess installs, and the pinned one is still there.
fn stockfish(fetch: &mut Fetch) -> Found {
    let (tag, assets) = release(fetch, sources::STOCKFISH.url)?;
    if !tag.starts_with("sf_") {
        return Err(format!(
            "Stockfish's latest release {tag} is no sf_ release"
        ));
    }
    let mut missing: Vec<&str> = SHIPPED
        .assets
        .iter()
        .map(|(_, _, name, _)| *name)
        .filter(|name| asset(&assets, name, sources::STOCKFISH_DOWNLOADS, &tag).is_none())
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if !missing.is_empty() {
        return Err(format!(
            "Stockfish's latest release {tag} has no {}",
            missing.join(", ")
        ));
    }
    let pinned = format!("{}{}", sources::STOCKFISH_TAGS, SHIPPED.tag);
    let (found, _) = release(fetch, &pinned)?;
    if found != SHIPPED.tag {
        return Err(format!("{pinned} answered with release {found}"));
    }
    Ok(format!("latest {tag}, pinned {}", SHIPPED.tag))
}

/// The speech model is the zip of the size KChess pins.
fn vosk_model(fetch: &mut Fetch) -> Found {
    let url = sources::VOSK_MODEL.url;
    let answer = fetch(&Request::head(url, 4))?;
    magic(&answer, is_zip, "a zip", url)?;
    if answer.total == Some(VOSK_MODEL_BYTES) {
        Ok(format!("{VOSK_MODEL_BYTES} bytes"))
    } else {
        Err(format!(
            "{url} is {} bytes, not the pinned model's {VOSK_MODEL_BYTES}",
            answer
                .total
                .map_or("an unknown number of".into(), |t| t.to_string())
        ))
    }
}

/// The smallest table files, whole, match the sizes and digests the core pins.
fn syzygy(fetch: &mut Fetch, tables: &str) -> Found {
    for name in TABLES {
        let (bytes, sha256) = tables
            .lines()
            .find_map(|line| {
                let mut parts = line.split_whitespace();
                (parts.next() == Some(name)).then(|| (parts.next(), parts.next()))
            })
            .and_then(|(bytes, sha256)| Some((bytes?.parse::<usize>().ok()?, sha256?)))
            .ok_or_else(|| format!("the core's files.txt does not pin {name}"))?;
        let folder = if name.ends_with(".rtbz") {
            "3-4-5-dtz"
        } else {
            "3-4-5-wdl"
        };
        let url = format!("{}/{folder}/{name}", sources::SYZYGY.url);
        let answer = fetch(&Request::get(&url))?;
        let found = kchess_delta::hex(&Sha256::digest(&answer.body));
        if answer.body.len() != bytes || found != sha256 {
            return Err(format!(
                "{url} is {} bytes with SHA-256 {found}, not the pinned {bytes} bytes with {sha256}",
                answer.body.len()
            ));
        }
    }
    Ok(TABLES.join(", "))
}

/// One player's archives of months, as the Chess.com sync reads them.
fn chesscom_games(fetch: &mut Fetch) -> Found {
    let player = format!(
        "{}/pub/player/{CHESSCOM_PLAYER}/games/",
        sources::CHESSCOM.url
    );
    let url = format!("{player}archives");
    let answer: serde_json::Value = serde_json::from_slice(&fetch(&Request::get(&url))?.body)
        .map_err(|_| format!("{url} answered with no JSON"))?;
    let archives = answer["archives"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let months = archives
        .iter()
        .filter(|archive| archive.as_str().is_some_and(|a| a.starts_with(&player)))
        .count();
    if months > 0 && months == archives.len() {
        Ok(format!("{months} months of {CHESSCOM_PLAYER}'s games"))
    } else {
        Err(format!("{url} answered with no months of games"))
    }
}

/// Each download's check, as `(source, what it found)`, in `sources::DOWNLOADS`' order.
fn check_all(fetch: &mut Fetch, known: &Known) -> Vec<(Source, Found)> {
    sources::DOWNLOADS
        .iter()
        .map(|source| {
            let found = match check_of(source) {
                Some(check) => check(fetch, known),
                None => Err("no check is written for it (source_check.rs `check_of`)".into()),
            };
            (*source, found)
        })
        .collect()
}

/// The report of `results`: a line a source, and the failures, each naming its source and owner.
fn report(results: &[(Source, Found)]) -> Result {
    let failed: Vec<String> = results
        .iter()
        .filter_map(|(source, found)| {
            found.as_ref().err().map(|why| {
                format!(
                    "{} ({}), run by {}: {why}.",
                    source.name, source.url, source.owner
                )
            })
        })
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} download sources no longer serve what KChess reads:\n{}",
            failed.len(),
            results.len(),
            failed.join("\n")
        ))
    }
}

/// curl's arguments for `request`: a deadline on the whole request and on connecting, a size cap,
/// HTTPS only (redirects included), and failure on an HTTP error.
fn curl_args(request: &Request, headers: &str, body: &str, token: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    args.extend([
        "--max-time".into(),
        MAX_SECONDS.to_string(),
        "--connect-timeout".into(),
        CONNECT_SECONDS.to_string(),
        "--max-filesize".into(),
        MAX_ANSWER.to_string(),
        "--user-agent".into(),
        USER_AGENT.into(),
        "--dump-header".into(),
        headers.into(),
        "--output".into(),
        body.into(),
    ]);
    if let Some(bytes) = request.range {
        args.extend(["--range".into(), format!("0-{}", bytes - 1)]);
    }
    if let Some(accept) = request.accept {
        args.extend(["--header".into(), format!("Accept: {accept}")]);
    }
    if token {
        // The token goes in on standard input, never on the command line.
        args.extend(["--config".into(), "-".into()]);
    }
    args.push(request.url.clone());
    args
}

/// The status and `Content-Range` total of the last answer in curl's dumped headers (a redirect's
/// come first).
fn last_answer(headers: &str) -> (Option<u16>, Option<u64>) {
    let last = headers
        .rmatch_indices("HTTP/")
        .find(|(at, _)| *at == 0 || headers[..*at].ends_with('\n'))
        .map_or(headers, |(at, _)| &headers[at..]);
    let status = last
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse().ok());
    let total = last.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-range")
            .then(|| value.trim().rsplit('/').next()?.parse().ok())?
    });
    (status, total)
}

/// GitHub's API token from the environment, for its higher rate limit on CI.
fn github_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|token| !token.is_empty()))
}

/// Asks with curl, within `MAX_SECONDS`.
fn curl(request: &Request) -> Result<Answer> {
    let dir = tempfile::tempdir().map_err(|cause| cause.to_string())?;
    let (headers, body) = (dir.path().join("headers"), dir.path().join("body"));
    let token = github_token().filter(|_| request.url.starts_with("https://api.github.com/"));
    let args = curl_args(
        request,
        &headers.to_string_lossy(),
        &body.to_string_lossy(),
        token.is_some(),
    );
    let mut child = Command::new("curl")
        .args(&args)
        .stdin(if token.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|cause| format!("Cannot run curl: {cause}"))?;
    if let (Some(token), Some(mut stdin)) = (token, child.stdin.take()) {
        writeln!(stdin, "header = \"Authorization: Bearer {token}\"")
            .map_err(|cause| format!("Cannot hand curl the GitHub token: {cause}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|cause| cause.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "{} could not be read: {}",
            request.url,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let headers = std::fs::read_to_string(&headers).unwrap_or_default();
    let (status, total) = last_answer(&headers);
    if request.range.is_some() && status != Some(206) {
        return Err(format!(
            "{} answered {} to a request for its first bytes, not 206",
            request.url,
            status.map_or("nothing".into(), |s| s.to_string())
        ));
    }
    let body = std::fs::read(&body).map_err(|cause| cause.to_string())?;
    Ok(Answer { body, total })
}

pub fn run(_args: &[String]) -> Result {
    let (year, month, _) = civil_date(unix_now());
    let known = Known {
        today: (year, month),
        tables: TABLE_FILES,
    };
    let results = check_all(&mut curl, &known);
    for (source, found) in &results {
        match found {
            Ok(evidence) => say(&format!("ok      {}: {evidence}", source.name)),
            Err(why) => say(&format!("FAILED  {}: {why}", source.name)),
        }
    }
    report(&results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const TODAY: (i64, u32) = (2026, 11);

    // Each source's answer as it came on 2026-11-26, cut to a few lines.
    const BROADCAST_LIST: &str = "\
https://database.lichess.org/broadcast/lichess_db_broadcast_2026-10.pgn.zst
https://database.lichess.org/broadcast/lichess_db_broadcast_2026-09.pgn.zst
https://database.lichess.org/broadcast/lichess_db_broadcast_2026-08.pgn.zst
";
    const BROADCAST_SUMS: &str = "\
edd008e921e495e0f176967eb81434004692c52ae1cd1fe20e3d03bde37d24fb  lichess_db_broadcast_2026-08.pgn.zst
5b363f1041d893c034f7e349bbe8b290c909b135eca116c0eb1a942bd1d08ae4  lichess_db_broadcast_2026-09.pgn.zst
1f6a7c3b4e5d6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b  lichess_db_broadcast_2026-10.pgn.zst
";
    const STANDARD_LIST: &str = "\
https://database.lichess.org/standard/lichess_db_standard_rated_2026-10.pgn.zst
https://database.lichess.org/standard/lichess_db_standard_rated_2026-09.pgn.zst
";
    const STANDARD_SUMS: &str = "\
aa40b3671fa3cf1072eb182892cd90b0e1e003a4a5943492f64b77e7f3fd1635  lichess_db_standard_rated_2026-09.pgn.zst
c136acdf343293c45252906fee91e3b561fb26a936979f52dbe04bb649a2fd86  lichess_db_standard_rated_2026-10.pgn.zst
";
    const LICHESS_GAME: &str = "\
[Event \"Rated blitz game\"]
[Site \"https://lichess.org/abcdEFGH\"]
[White \"thibault\"]
[Black \"someone\"]
[Result \"1-0\"]

1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# 1-0
";
    const TCEC_RELEASE: &str = r#"{"tag_name":"S29-final","assets":[
        {"name":"ChangeLog.txt","digest":"sha256:84ef1ca52bc9216695da30987a23d4fb701e579985c34245c4045c076d8ec7de",
         "browser_download_url":"https://github.com/TCEC-Chess/tcecgames/releases/download/S29-final/ChangeLog.txt"},
        {"name":"TCEC-everything-compact.zip","digest":"sha256:4584f615b9ed098bcf37e38e740e66bff0b89143b3aad858713d98539a30b019",
         "browser_download_url":"https://github.com/TCEC-Chess/tcecgames/releases/download/S29-final/TCEC-everything-compact.zip"}]}"#;
    const CHESSCOM_ARCHIVES: &str = r#"{"archives":[
        "https://api.chess.com/pub/player/hikaru/games/2014/01",
        "https://api.chess.com/pub/player/hikaru/games/2026/11"]}"#;

    /// Stockfish's release `tag`, with every file the latest had on 2026-11-26.
    fn stockfish_release(tag: &str) -> Vec<u8> {
        let names = [
            "stockfish-android-arm64-universal.tar.gz",
            "stockfish-linux-arm64-universal.tar.gz",
            "stockfish-linux-x86-64-universal.tar.gz",
            "stockfish-macos-universal.tar.gz",
            "stockfish-windows-arm64-universal.zip",
            "stockfish-windows-x86-64-universal.zip",
        ];
        let assets: Vec<_> = names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("{}{tag}/{name}", sources::STOCKFISH_DOWNLOADS),
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "tag_name": tag, "assets": assets })).unwrap()
    }

    /// Stand-ins for the two table files, and the `files.txt` lines that pin them.
    fn tables() -> (Vec<u8>, Vec<u8>, String) {
        let (wdl, dtz) = (b"wdl table".to_vec(), b"dtz table!".to_vec());
        let pin = |name: &str, bytes: &[u8]| {
            format!(
                "{name} {} {}\n",
                bytes.len(),
                kchess_delta::hex(&Sha256::digest(bytes))
            )
        };
        let pins = format!(
            "KQvB.rtbw 1 00\n{}{}",
            pin("KQvK.rtbw", &wdl),
            pin("KQvK.rtbz", &dtz)
        );
        (wdl, dtz, pins)
    }

    /// Every source as it answered on 2026-11-26, by address.
    fn served() -> HashMap<String, Answer> {
        let answer = |body: &[u8], total: Option<u64>| Answer {
            body: body.to_vec(),
            total,
        };
        let (wdl, dtz, _) = tables();
        let broadcast = sources::LICHESS_BROADCAST.url;
        let standard = sources::LICHESS_STANDARD.url;
        HashMap::from([
            (
                format!("{broadcast}list.txt"),
                answer(BROADCAST_LIST.as_bytes(), None),
            ),
            (
                format!("{broadcast}sha256sums.txt"),
                answer(BROADCAST_SUMS.as_bytes(), None),
            ),
            (
                format!("{broadcast}lichess_db_broadcast_2026-10.pgn.zst"),
                answer(&ZSTD, Some(31_000_000)),
            ),
            (
                format!("{standard}list.txt"),
                answer(STANDARD_LIST.as_bytes(), None),
            ),
            (
                format!("{standard}sha256sums.txt"),
                answer(STANDARD_SUMS.as_bytes(), None),
            ),
            (
                format!("{standard}lichess_db_standard_rated_2026-10.pgn.zst"),
                answer(&ZSTD, Some(33_000_000_000)),
            ),
            (
                sources::LICHESS_PUZZLES.url.into(),
                answer(&ZSTD_SKIPPABLE, Some(309_751_173)),
            ),
            (
                format!("{}/api/games/user/thibault?max=1", sources::LICHESS.url),
                answer(LICHESS_GAME.as_bytes(), None),
            ),
            (
                sources::TCEC.url.into(),
                answer(TCEC_RELEASE.as_bytes(), None),
            ),
            (
                format!(
                    "{}S29-final/{}",
                    sources::TCEC_DOWNLOADS,
                    sources::TCEC_ASSET
                ),
                answer(&ZIP, Some(26_708_674)),
            ),
            (
                sources::STOCKFISH.url.into(),
                answer(&stockfish_release("sf_19"), None),
            ),
            (
                format!("{}{}", sources::STOCKFISH_TAGS, SHIPPED.tag),
                answer(&stockfish_release(SHIPPED.tag), None),
            ),
            (
                sources::VOSK_MODEL.url.into(),
                answer(&ZIP, Some(VOSK_MODEL_BYTES)),
            ),
            (
                format!("{}/3-4-5-wdl/KQvK.rtbw", sources::SYZYGY.url),
                answer(&wdl, None),
            ),
            (
                format!("{}/3-4-5-dtz/KQvK.rtbz", sources::SYZYGY.url),
                answer(&dtz, None),
            ),
            (
                format!("{}/pub/player/hikaru/games/archives", sources::CHESSCOM.url),
                answer(CHESSCOM_ARCHIVES.as_bytes(), None),
            ),
        ])
    }

    /// Checks every source against `served`, as of `TODAY`.
    fn check_served(
        served: &HashMap<String, Answer>,
        asked: &mut Vec<Request>,
    ) -> Vec<(Source, Found)> {
        let (_, _, pins) = tables();
        let known = Known {
            today: TODAY,
            tables: &pins,
        };
        check_all(&mut from(served, asked), &known)
    }

    /// Asks `served`, recording each request; an address it does not serve is a 404.
    fn from(
        served: &HashMap<String, Answer>,
        asked: &mut Vec<Request>,
    ) -> impl FnMut(&Request) -> Result<Answer> {
        move |request| {
            asked.push(request.clone());
            served
                .get(&request.url)
                .cloned()
                .ok_or_else(|| format!("{} could not be read: 404", request.url))
        }
    }

    fn failures(served: &HashMap<String, Answer>) -> Vec<(&'static str, String)> {
        check_served(served, &mut Vec::new())
            .into_iter()
            .filter_map(|(source, found)| Some((source.name, found.err()?)))
            .collect()
    }

    #[test]
    fn every_source_as_served_passes_asking_only_small_parts() {
        let served = served();
        let mut asked = Vec::new();
        let results = check_served(&served, &mut asked);
        assert!(report(&results).is_ok(), "{:?}", report(&results));
        assert_eq!(results.len(), sources::DOWNLOADS.len());
        for request in &asked {
            let big = [".pgn.zst", ".csv.zst", ".zip"];
            if big.iter().any(|end| request.url.ends_with(end)) {
                assert_eq!(request.range, Some(4), "{} is asked whole", request.url);
            }
        }
    }

    #[test]
    fn every_download_has_a_check_and_every_source_is_listed() {
        for source in sources::DOWNLOADS {
            assert!(check_of(source).is_some(), "{} has no check", source.name);
        }
        for source in sources::SERVICES {
            assert!(
                check_of(source).is_none(),
                "{} is checked but listed as a service",
                source.name
            );
        }
        let text = include_str!("../../../crates/kchess-core/src/sources.rs");
        let declared = text.matches(": Source = Source {").count();
        assert_eq!(
            declared,
            sources::DOWNLOADS.len() + sources::SERVICES.len(),
            "each Source in sources.rs is in DOWNLOADS or SERVICES"
        );
    }

    /// Every `"http(s)://…"` literal in the core's shipped code outside `sources.rs`, as
    /// `path:line`, except a local address or a bare scheme.
    fn addresses_outside_sources(files: &[(String, String)]) -> Vec<String> {
        let local = [
            "http://127.0.0.1",
            "http://localhost",
            "http://{",
            "http://[",
        ];
        let mut found = Vec::new();
        for (path, text) in files {
            if crate::boundaries::is_test_file(path) || path.ends_with("/sources.rs") {
                continue;
            }
            for (index, line) in crate::boundaries::shipped(text).lines().enumerate() {
                for (at, _) in line.match_indices("\"http") {
                    let literal = &line[at + 1..];
                    let rest = literal
                        .strip_prefix("https://")
                        .or_else(|| literal.strip_prefix("http://"));
                    let named = rest.is_some_and(|rest| !rest.starts_with('"'));
                    if named && !local.iter().any(|local| literal.starts_with(local)) {
                        found.push(format!("{path}:{}", index + 1));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn every_third_party_address_is_a_source() {
        let mut files = Vec::new();
        crate::util::sources_under(&crate::util::root(), "crates/kchess-core/src", &mut files)
            .unwrap();
        assert!(files.len() > 50);
        let found = addresses_outside_sources(&files);
        assert!(
            found.is_empty(),
            "Third-party addresses belong in crates/kchess-core/src/sources.rs, with their owner \
             (and a source-check for a download):\n{}",
            found.join("\n")
        );
    }

    /// Windows checks the sources out with CRLF line endings, where the scan once took every
    /// test module for shipped code.
    #[test]
    fn the_address_scan_reads_a_windows_checkout_alike() {
        let mut files = Vec::new();
        crate::util::sources_under(&crate::util::root(), "crates/kchess-core/src", &mut files)
            .unwrap();
        let lf = addresses_outside_sources(&files);
        let crlf: Vec<(String, String)> = files
            .iter()
            .map(|(path, text)| {
                (
                    path.clone(),
                    text.replace("\r\n", "\n").replace('\n', "\r\n"),
                )
            })
            .collect();
        assert_eq!(addresses_outside_sources(&crlf), lf);
    }

    #[test]
    fn the_address_scan_finds_a_literal_but_not_a_local_one_a_scheme_or_a_test() {
        let file = |path: &str, text: &str| (path.to_string(), text.to_string());
        let files = [
            file(
                "crates/kchess-core/src/a.rs",
                "const A: &str = \"https://elsewhere.org/x\";\n",
            ),
            file(
                "crates/kchess-core/src/b.rs",
                "let a = format!(\"http://127.0.0.1:{port}\");\nif u.starts_with(\"https://\") {}\n",
            ),
            file(
                "crates/kchess-core/src/c/tests.rs",
                "\"https://example.org\"\n",
            ),
            file(
                "crates/kchess-core/src/d.rs",
                "fn a() {}\n#[cfg(test)]\nmod tests {\n \"https://example.org\"\n}\n",
            ),
            file(
                "crates/kchess-core/src/sources.rs",
                "\"https://lichess.org\"\n",
            ),
        ];
        assert_eq!(
            addresses_outside_sources(&files),
            ["crates/kchess-core/src/a.rs:1"]
        );
    }

    #[test]
    fn an_archive_lists_its_months_newest_first() {
        let list = "https://database.lichess.org/broadcast/lichess_db_broadcast_2026-09.pgn.zst\n\
                    https://database.lichess.org/broadcast/lichess_db_broadcast_2026-10.pgn.zst\n\
                    https://elsewhere.example/lichess_db_broadcast_2026-11.pgn.zst\n\
                    https://database.lichess.org/broadcast/lichess_db_broadcast_2026-13.pgn.zst\n\
                    https://database.lichess.org/broadcast/notes.txt\n";
        assert_eq!(
            listed_months(
                list,
                sources::LICHESS_BROADCAST.url,
                "lichess_db_broadcast_"
            ),
            ["2026-10", "2026-09"]
        );
        assert_eq!(months_before("2026-10", TODAY), Some(1));
        assert_eq!(months_before("2025-12", (2026, 2)), Some(2));
    }

    #[test]
    fn a_silent_archive_fails_naming_its_last_month() {
        let mut served = served();
        let list = format!("{}list.txt", sources::LICHESS_BROADCAST.url);
        served.get_mut(&list).unwrap().body = format!(
            "{}lichess_db_broadcast_2026-07.pgn.zst\n",
            sources::LICHESS_BROADCAST.url
        )
        .into_bytes();
        let found = failures(&served);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Lichess broadcast games");
        assert!(
            found[0]
                .1
                .contains("no new month since 2026-07, 4 months ago"),
            "{}",
            found[0].1
        );
    }

    #[test]
    fn a_hijacked_domain_fails_every_check_it_serves() {
        // A parked or taken-over domain answers every address with the same page.
        let page = Answer {
            body: b"<!doctype html><title>Best online casino</title>".to_vec(),
            total: Some(48),
        };
        let mut served = served();
        for (url, answer) in served.iter_mut() {
            if url.starts_with("https://database.lichess.org/")
                || url.starts_with("https://alphacephei.com/")
            {
                *answer = page.clone();
            }
        }
        let names: Vec<_> = failures(&served)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [
                "Lichess broadcast games",
                "Lichess rated games",
                "Lichess puzzles",
                "Vosk English speech model"
            ]
        );
    }

    #[test]
    fn a_month_without_a_digest_or_a_file_that_is_not_zstd_fails() {
        let mut served = served();
        let sums = format!("{}sha256sums.txt", sources::LICHESS_STANDARD.url);
        let text = String::from_utf8(served[&sums].body.clone()).unwrap();
        let kept: String = text
            .lines()
            .filter(|l| !l.contains("2026-10"))
            .map(|l| format!("{l}\n"))
            .collect();
        served.get_mut(&sums).unwrap().body = kept.into_bytes();
        let month = format!(
            "{}lichess_db_broadcast_2026-10.pgn.zst",
            sources::LICHESS_BROADCAST.url
        );
        served.get_mut(&month).unwrap().body = b"<htm".to_vec();
        let found = failures(&served);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].1.ends_with("does not start as a zstd file does"),
            "{}",
            found[0].1
        );
        assert!(
            found[1]
                .1
                .ends_with("has no SHA-256 for lichess_db_standard_rated_2026-10.pgn.zst"),
            "{}",
            found[1].1
        );
    }

    #[test]
    fn a_release_without_the_asset_or_its_digest_fails() {
        let mut served = served();
        let strip = |served: &mut HashMap<String, Answer>,
                     url: &str,
                     edit: &dyn Fn(&mut serde_json::Value)| {
            let mut release: serde_json::Value = serde_json::from_slice(&served[url].body).unwrap();
            edit(&mut release);
            served.get_mut(url).unwrap().body = serde_json::to_vec(&release).unwrap();
        };
        strip(&mut served, sources::TCEC.url, &|release| {
            for asset in release["assets"].as_array_mut().unwrap() {
                asset["digest"] = serde_json::Value::Null;
            }
        });
        strip(&mut served, sources::STOCKFISH.url, &|release| {
            release["assets"]
                .as_array_mut()
                .unwrap()
                .retain(|asset| asset["name"] != "stockfish-macos-universal.tar.gz");
        });
        let found = failures(&served);
        assert_eq!(
            found,
            [
                (
                    "TCEC engine games",
                    "TCEC-everything-compact.zip of TCEC's release S29-final has no SHA-256"
                        .to_string()
                ),
                (
                    "Stockfish",
                    "Stockfish's latest release sf_19 has no stockfish-macos-universal.tar.gz"
                        .to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_changed_file_or_an_address_gone_fails_naming_the_source_and_owner() {
        let mut served = served();
        served.get_mut(sources::VOSK_MODEL.url).unwrap().total = Some(1_000);
        let table = format!("{}/3-4-5-wdl/KQvK.rtbw", sources::SYZYGY.url);
        served.get_mut(&table).unwrap().body.push(b'!');
        served.remove(&format!(
            "{}/pub/player/hikaru/games/archives",
            sources::CHESSCOM.url
        ));
        served.remove(&format!(
            "{}/api/games/user/thibault?max=1",
            sources::LICHESS.url
        ));
        let mut asked = Vec::new();
        let results = check_served(&served, &mut asked);
        let report = report(&results).unwrap_err();
        assert!(
            report.starts_with("4 of 9 download sources no longer serve what KChess reads:"),
            "{report}"
        );
        assert!(
            report.contains("Lichess (https://lichess.org), run by Lichess (lichess.org"),
            "{report}"
        );
        assert!(
            report.contains("is 1000 bytes, not the pinned model's 41205931"),
            "{report}"
        );
        assert!(report.contains("KQvK.rtbw is 10 bytes"), "{report}");
        assert!(
            report.contains("Chess.com games (https://api.chess.com), run by Chess.com"),
            "{report}"
        );
    }

    #[test]
    fn curl_has_a_deadline_https_only_and_the_token_off_the_command_line() {
        let request = Request {
            range: Some(4),
            accept: Some("application/x-chess-pgn"),
            ..Request::get("https://example.org/a")
        };
        let args = curl_args(&request, "h", "b", true);
        let after = |flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone();
        assert_eq!(after("--max-time"), "60");
        assert_eq!(after("--connect-timeout"), "20");
        assert_eq!(after("--proto"), "=https");
        assert_eq!(after("--proto-redir"), "=https");
        assert_eq!(after("--range"), "0-3");
        assert_eq!(after("--config"), "-");
        assert!(args.contains(&"--fail".to_string()));
        assert_eq!(args.last().unwrap(), "https://example.org/a");
        assert!(!args.join(" ").contains("Bearer"));
        assert!(
            !curl_args(&Request::get("https://x.org"), "h", "b", false)
                .contains(&"--config".to_string())
        );
    }

    #[test]
    fn the_last_answer_after_redirects_gives_the_status_and_size() {
        let headers = "HTTP/2 302 \r\nlocation: https://objects.example/a\r\ncontent-range: bytes 0-3/9\r\n\r\n\
                       HTTP/1.1 206 Partial Content\r\nContent-Type: application/zip\r\nContent-Range: bytes 0-3/41205931\r\n\r\n";
        assert_eq!(last_answer(headers), (Some(206), Some(41_205_931)));
        assert_eq!(
            last_answer("HTTP/2 200 \r\ncontent-type: text/plain\r\n\r\n"),
            (Some(200), None)
        );
        assert_eq!(last_answer(""), (None, None));
    }
}
