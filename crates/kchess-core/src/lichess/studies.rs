//! Lichess studies (`core/src/services/studies.ts`): the study list of an account, the chapters of
//! a study, export of a game into a study, and the explicit cloud sync that refuses to overwrite
//! concurrent cloud edits.
//!
//! Every cloud call runs through `Lichess::as_account`, so a missing or refused login answers
//! `Reply::NeedsReconnect` instead of failing. A failed mutation is never retried.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::{Failure, authorize};
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::{js, position};

/// `MAX_STUDY_PGN`: the most UTF-16 units of a study export that are read.
const MAX_STUDY_PGN: usize = 4_000_000;
/// Most chapters a study sync or import handles.
const MAX_CHAPTERS: usize = 64;
/// Most studies listed for an account.
const MAX_STUDIES: usize = 300;

/// A study the account owns or belongs to (`LichessStudy`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LichessStudy {
    pub id: String,
    pub name: String,
    pub updated_at: i64,
}

/// A chapter of a study, with its PGN (`LichessStudyChapter`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LichessStudyChapter {
    pub name: String,
    pub pgn: String,
}

/// The upload a user makes of their edits to a linked study (`StudySyncRequest`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudySyncRequest {
    pub account: String,
    pub study_id: String,
    pub baseline: String,
    pub pgn: String,
}

/// `lichessStudies(account)`: the studies the account owns or belongs to (private ones too, with
/// `study:read`), most recently updated first.
pub async fn lichess_studies(
    lichess: &Lichess,
    account: &str,
) -> CoreResult<Reply<Vec<LichessStudy>>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(account, "study", |token| async move {
            let auth = authorize(&token);
            let path = format!("/api/study/by/{}", path_segment(account));
            // Each record is read as it arrives: a malformed one ends the listing at once.
            let mut studies = Vec::new();
            lichess
                .client()
                .ndjson(
                    reqwest::Method::GET,
                    &path,
                    &[],
                    None,
                    Some(auth.as_str()),
                    &cancel,
                    |line| {
                        if let Some(study) = study_metadata(line)?
                            && studies.len() < MAX_STUDIES
                        {
                            studies.push(study);
                        }
                        Ok(())
                    },
                )
                .await?;
            studies.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            Ok(studies)
        })
        .await
}

/// A study's metadata line, when it has a valid id and name (`metadata` of the schema).
fn study_metadata(line: &str) -> Result<Option<LichessStudy>, Failure> {
    let raw: Value = serde_json::from_str(line).map_err(|cause| {
        Failure::Core(CoreError::new(format!(
            "Lichess sent an unreadable study record: {cause}"
        )))
    })?;
    let id = raw.get("id").and_then(Value::as_str);
    let name = raw.get("name").and_then(Value::as_str);
    let optional_number = |key: &str| -> Option<Option<i64>> {
        match raw.get(key) {
            None => Some(None),
            Some(value) => value.as_i64().map(Some).or_else(|| {
                value
                    .as_f64()
                    .filter(|n| n.fract() == 0.0)
                    .map(|n| Some(n as i64))
            }),
        }
    };
    let (Some(id), Some(name)) = (id, name) else {
        return Ok(None);
    };
    let valid = id.len() == 8
        && id.bytes().all(|b| b.is_ascii_alphanumeric())
        && js::utf16_len(name) <= 200;
    let (Some(created), Some(updated)) =
        (optional_number("createdAt"), optional_number("updatedAt"))
    else {
        return Ok(None);
    };
    if !valid {
        return Ok(None);
    }
    Ok(Some(LichessStudy {
        id: id.to_string(),
        name: name.to_string(),
        updated_at: updated.or(created).unwrap_or(0),
    }))
}

/// `splitPgn(text)`: a multi-game PGN split into games. Lichess separates games with blank lines
/// before a tag; each part is trimmed and kept only when it starts with a tag.
pub fn split_pgn(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n");
    let chars: Vec<char> = text.chars().collect();
    let count = chars.len();
    let mut pieces: Vec<String> = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < count {
        if chars[index] == '\n' {
            // `\n\s*\n(?=\[)`: the second newline is the last one before the tag.
            let mut end = index + 1;
            while end < count && js::is_space(chars[end]) {
                end += 1;
            }
            if end < count && chars[end] == '[' && end - 1 > index && chars[end - 1] == '\n' {
                pieces.push(chars[start..index].iter().collect());
                start = end;
                index = end;
                continue;
            }
        }
        index += 1;
    }
    pieces.push(chars[start..].iter().collect());
    pieces
        .into_iter()
        .map(|piece| js::trim(&piece).to_string())
        .filter(|piece| piece.starts_with('['))
        .collect()
}

/// `/^\[NAME "([^"]*)"\]/m`: the value of the first tag line named `name`.
fn tag_value(pgn: &str, name: &str) -> Option<String> {
    let prefix = format!("[{name} \"");
    let mut line_start = 0;
    loop {
        if pgn[line_start..].starts_with(&prefix) {
            let value_start = line_start + prefix.len();
            if let Some(quote) = pgn[value_start..].find('"') {
                let end = value_start + quote;
                if pgn[end + 1..].starts_with(']') {
                    return Some(pgn[value_start..end].to_string());
                }
            }
        }
        match pgn[line_start..].find('\n') {
            Some(newline) => line_start += newline + 1,
            None => return None,
        }
    }
}

/// The first `units` UTF-16 units of `text` (JavaScript's `slice(0, units)`).
fn take_utf16(text: &str, units: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let width = c.len_utf16();
        if used + width > units {
            break;
        }
        used += width;
        out.push(c);
    }
    out
}

/// `lichessStudyChapters(account, id)`: every chapter of a study, with variations, comments and
/// NAGs, ready for the analysis board.
pub async fn lichess_study_chapters(
    lichess: &Lichess,
    account: &str,
    id: &str,
) -> CoreResult<Reply<Vec<LichessStudyChapter>>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(account, "study", |token| async move {
            let auth = authorize(&token);
            let path = format!("/api/study/{}.pgn", path_segment(id));
            let query = [
                ("comments", "true".to_string()),
                ("variations", "true".to_string()),
                ("clocks", "false".to_string()),
                ("orientation", "true".to_string()),
            ];
            let text = lichess
                .client()
                .get_text(
                    &path,
                    &query,
                    "application/x-chess-pgn",
                    Some(auth.as_str()),
                    &cancel,
                )
                .await?;
            if js::utf16_len(&text) > MAX_STUDY_PGN {
                return Err(CoreError::new("That study is too large to import.").into());
            }
            Ok(split_pgn(&text)
                .into_iter()
                .take(MAX_CHAPTERS)
                .enumerate()
                .map(|(index, pgn)| {
                    let name = tag_value(&pgn, "ChapterName")
                        .or_else(|| tag_value(&pgn, "Event"))
                        .unwrap_or_else(|| format!("Chapter {}", index + 1));
                    LichessStudyChapter {
                        name: take_utf16(&name, 120),
                        pgn,
                    }
                })
                .collect())
        })
        .await
}

/// `exportToLichessStudy(account, studyId, name, pgn)`: adds a game to a Lichess study as new
/// chapters, or to a new private study when `study_id` is empty. Returns the study id.
pub async fn export_to_lichess_study(
    lichess: &Lichess,
    account: &str,
    study_id: &str,
    name: &str,
    pgn: &str,
) -> CoreResult<Reply<String>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(account, "study", |token| async move {
            let auth = authorize(&token);
            let mut id = study_id.to_string();
            if id.is_empty() {
                let created: Value = lichess
                    .client()
                    .post_form(
                        "/api/study",
                        &[
                            ("name", name),
                            ("visibility", "private"),
                            ("computer", "everyone"),
                            ("explorer", "everyone"),
                            ("cloneable", "owner"),
                            ("shareable", "owner"),
                            ("chat", "member"),
                        ],
                        Some(auth.as_str()),
                        &cancel,
                    )
                    .await?;
                id = created
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| CoreError::new("Lichess did not create the study."))?;
            }
            let path = format!("/api/study/{}/import-pgn", path_segment(&id));
            let _: Value = lichess
                .client()
                .post_form(
                    &path,
                    &[("pgn", pgn), ("name", name)],
                    Some(auth.as_str()),
                    &cancel,
                )
                .await?;
            Ok(id)
        })
        .await
}

/// `syncLichessStudy(request)`: uploads edits to a linked study. The cloud copy must still match
/// the baseline the user downloaded; otherwise nothing is written and the user is told so. Only
/// changed chapters are updated, and new chapters are appended. Returns the study's chapters
/// after the upload.
pub async fn sync_lichess_study(
    lichess: &Lichess,
    request: &StudySyncRequest,
) -> CoreResult<Reply<Vec<LichessStudyChapter>>> {
    let before = valid_chapters(&request.baseline)?;
    let after = valid_chapters(&request.pgn)?;
    if before.is_empty() || before.len() > after.len() || before.len() > MAX_CHAPTERS {
        return Err(CoreError::new(
            "Chapter structure changed. Upload a new cloud copy instead.",
        ));
    }
    let current = match lichess_study_chapters(lichess, &request.account, &request.study_id).await?
    {
        Reply::Done(chapters) => chapters,
        Reply::NeedsReconnect => return Ok(Reply::NeedsReconnect),
    };
    let joined: Vec<String> = current.iter().map(|chapter| chapter.pgn.clone()).collect();
    let remote = valid_chapters(&joined.join("\n\n"))?;
    let changed_remotely = remote.len() != before.len()
        || remote
            .iter()
            .zip(before.iter())
            .any(|(game, expected)| game.pgn != expected.pgn);
    if changed_remotely {
        return Err(CoreError::new(
            "The cloud study changed. Download its latest copy before uploading; your offline edits are kept.",
        ));
    }
    let mut ids = Vec::with_capacity(before.len());
    for game in &before {
        let site = map_get(&game.headers, "Site").unwrap_or("");
        match study_chapter_id(site) {
            Some((study, chapter)) if study == request.study_id => ids.push(chapter),
            _ => {
                return Err(CoreError::new(
                    "Missing cloud chapter identity. Download the study again first.",
                ));
            }
        }
    }
    let cancel = lichess.lifetime().clone();
    let result = lichess
        .as_account(&request.account, "study", |token| async move {
            let auth = authorize(&token);
            for (index, expected) in before.iter().enumerate() {
                let game = &after[index];
                if game.pgn == expected.pgn {
                    continue;
                }
                let base = format!(
                    "/api/study/{}/{}",
                    path_segment(&request.study_id),
                    path_segment(&ids[index])
                );
                let _: Value = lichess
                    .client()
                    .post_form(
                        &format!("{base}/moves"),
                        &[("pgn", game.pgn.as_str())],
                        Some(auth.as_str()),
                        &cancel,
                    )
                    .await?;
                let mut tags = game.headers.clone();
                // Chapter identity belongs to Lichess, rather than an edited local PGN.
                let site = map_get(&expected.headers, "Site").unwrap_or("").to_string();
                map_set(&mut tags, "Site", &site);
                // Removing a local PGN tag must also remove it from the cloud chapter.
                for (key, _) in &expected.headers {
                    if map_get(&tags, key).is_none() {
                        map_set(&mut tags, key, "");
                    }
                }
                let tags_pgn = tags
                    .iter()
                    .map(|(key, value)| {
                        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                        format!("[{key} \"{escaped}\"]")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let _: Value = lichess
                    .client()
                    .post_form(
                        &format!("{base}/tags"),
                        &[("pgn", tags_pgn.as_str())],
                        Some(auth.as_str()),
                        &cancel,
                    )
                    .await?;
            }
            for game in after.iter().skip(before.len()) {
                let name = map_get(&game.headers, "ChapterName")
                    .or_else(|| map_get(&game.headers, "Event"))
                    .unwrap_or("New chapter")
                    .to_string();
                let path = format!("/api/study/{}/import-pgn", path_segment(&request.study_id));
                let _: Value = lichess
                    .client()
                    .post_form(
                        &path,
                        &[("pgn", game.pgn.as_str()), ("name", name.as_str())],
                        Some(auth.as_str()),
                        &cancel,
                    )
                    .await?;
            }
            Ok(())
        })
        .await?;
    match result {
        Reply::NeedsReconnect => Ok(Reply::NeedsReconnect),
        Reply::Done(()) => {
            lichess_study_chapters(lichess, &request.account, &request.study_id).await
        }
    }
}

/// `[Site]` of a cloud chapter: `…/study/{study}/{chapter}`, optionally followed by `?` or `#`.
fn study_chapter_id(site: &str) -> Option<(String, String)> {
    let mut from = 0;
    while let Some(at) = site[from..].find("/study/") {
        let start = from + at + "/study/".len();
        from = start;
        let Some(study) = site.get(start..start + 8) else {
            continue;
        };
        let Some(rest) = site.get(start + 8..).and_then(|r| r.strip_prefix('/')) else {
            continue;
        };
        let Some(chapter) = rest.get(..8) else {
            continue;
        };
        let tail = &rest[8..];
        let tail_ok = tail.is_empty() || tail.starts_with('?') || tail.starts_with('#');
        let ids_ok = study.bytes().all(|b| b.is_ascii_alphanumeric())
            && chapter.bytes().all(|b| b.is_ascii_alphanumeric());
        if ids_ok && tail_ok {
            return Some((study.to_string(), chapter.to_string()));
        }
    }
    None
}

/// A chapter with its headers as a JavaScript `Map` keeps them (first key order, last value).
#[derive(Clone, Debug, PartialEq)]
struct Chapter {
    headers: Vec<(String, String)>,
    pgn: String,
}

/// `validChapters(pgn)`: the games of a PGN, each replayable from its start and with legal moves.
fn valid_chapters(pgn: &str) -> CoreResult<Vec<Chapter>> {
    let Some(games) = position::pgn_games(pgn).as_array().cloned() else {
        return Err(CoreError::new("Use up to 64 valid chapters."));
    };
    if games.is_empty() || games.len() > MAX_CHAPTERS {
        return Err(CoreError::new("Use up to 64 valid chapters."));
    }
    let mut chapters = Vec::with_capacity(games.len());
    for game in &games {
        match game.get("problem").and_then(Value::as_str) {
            Some("start") => return Err(CoreError::new("Invalid chapter starting position.")),
            Some("move") => return Err(CoreError::new("The chapter contains an illegal move.")),
            _ => {}
        }
        let pairs = game
            .get("headers")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|pair| {
                        let pair = pair.as_array()?;
                        Some((
                            pair.first()?.as_str()?.to_string(),
                            pair.get(1)?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut headers = Vec::new();
        for (key, value) in pairs {
            map_set(&mut headers, &key, &value);
        }
        chapters.push(Chapter {
            headers,
            pgn: game
                .get("pgn")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(chapters)
}

/// `Map.prototype.get`.
fn map_get<'a>(map: &'a [(String, String)], key: &str) -> Option<&'a str> {
    map.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// `Map.prototype.set`: replaces the value in place, or appends a new key.
fn map_set(map: &mut Vec<(String, String)>, key: &str, value: &str) {
    match map.iter_mut().find(|(name, _)| name == key) {
        Some(entry) => entry.1 = value.to_string(),
        None => map.push((key.to_string(), value.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_study_into_games() {
        let pgn = "[Event \"Test Open\"]\n\n1. e4 *";
        let text = format!("{pgn}\n\n\n{}\n", pgn.replace("Test Open", "Second"));
        assert_eq!(split_pgn(&text).len(), 2);
    }

    #[test]
    fn finds_chapter_site_ids_only_in_study_urls() {
        assert_eq!(
            study_chapter_id("https://lichess.org/study/Study001/Chapter1"),
            Some(("Study001".into(), "Chapter1".into()))
        );
        assert_eq!(
            study_chapter_id("https://lichess.org/study/Study001/Chapter1?x=1"),
            Some(("Study001".into(), "Chapter1".into()))
        );
        assert_eq!(study_chapter_id("https://lichess.org/study/Study001"), None);
    }
}
