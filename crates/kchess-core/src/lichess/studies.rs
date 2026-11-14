//! Lichess studies: the study list of an account, the chapters of
//! a study, export of a game into a study, and the explicit cloud sync that refuses to overwrite
//! concurrent cloud edits.
//!
//! Every cloud call runs through `Lichess::as_account`, so a missing or refused login answers
//! `Reply::NeedsReconnect` instead of failing. A failed mutation is never retried.

use serde_json::Value;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::TextLimit;
use kchess_contracts::error::{CoreError, Result as CoreResult};
use kchess_contracts::types::{ChapterInput, LichessStudy, StudySyncRequest, UsageKind};
use kchess_domain::{js, position};

/// The most of a study export that is read, in UTF-16 units.
const STUDY_PGN_LIMIT: TextLimit<'static> = TextLimit {
    utf16_units: 4_000_000,
    message: "That study is too large to import.",
};
/// Most chapters a study sync or import handles.
const MAX_CHAPTERS: usize = 64;
/// Most studies listed for an account.
const MAX_STUDIES: usize = 300;

/// The studies the account owns or belongs to (private ones too, with
/// `study:read`), most recently updated first.
pub async fn lichess_studies(
    lichess: &Lichess,
    account: &str,
) -> CoreResult<Reply<Vec<LichessStudy>>> {
    lichess
        .as_account(account, UsageKind::Study, |auth| async move {
            let path = format!("/api/study/by/{}", path_segment(account));
            // Each record is read as it arrives: a malformed one ends the listing at once.
            let mut studies = lichess
                .client()
                .get(&path, &auth)
                .records("study", MAX_STUDIES, |raw: Value| study_metadata(&raw))
                .await?;
            studies.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
            Ok(studies)
        })
        .await
}

/// A study's metadata line, when it has a valid id and name (`metadata` of the schema).
fn study_metadata(raw: &Value) -> Option<LichessStudy> {
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
        return None;
    };
    let valid = super::is_lichess_id(id) && js::utf16_len(name) <= 200;
    let (Some(created), Some(updated)) =
        (optional_number("createdAt"), optional_number("updatedAt"))
    else {
        return None;
    };
    if !valid {
        return None;
    }
    Some(LichessStudy {
        id: id.to_string(),
        name: name.to_string(),
        updated_at: updated.or(created).unwrap_or(0),
    })
}

/// A multi-game PGN split into games. Lichess separates games with blank lines
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
        {
            let newline = pgn[line_start..].find('\n')?;
            line_start += newline + 1
        }
    }
}

/// Every chapter of a study, with variations, comments and
/// NAGs, ready for the analysis board.
pub async fn lichess_study_chapters(
    lichess: &Lichess,
    account: &str,
    id: &str,
) -> CoreResult<Reply<Vec<ChapterInput>>> {
    lichess
        .as_account(account, UsageKind::Study, |auth| async move {
            let path = format!("/api/study/{}.pgn", path_segment(id));
            let query = [
                ("comments", "true".to_string()),
                ("variations", "true".to_string()),
                ("clocks", "false".to_string()),
                ("orientation", "true".to_string()),
            ];
            let text = lichess
                .client()
                .get(&path, &auth)
                .query(&query)
                .accept("application/x-chess-pgn")
                .read_text(Some(STUDY_PGN_LIMIT))
                .await?;
            Ok(split_pgn(&text)
                .into_iter()
                .take(MAX_CHAPTERS)
                .enumerate()
                .map(|(index, pgn)| {
                    let name = tag_value(&pgn, "ChapterName")
                        .or_else(|| tag_value(&pgn, "Event"))
                        .unwrap_or_else(|| format!("Chapter {}", index + 1));
                    ChapterInput {
                        name: js::slice_utf16(&name, 120).to_string(),
                        pgn,
                    }
                })
                .collect())
        })
        .await
}

/// One chapter of a public study, as a study link names it
/// (its first when `chapter` is empty), with variations, comments and NAGs; no login needed.
pub async fn public_study_chapter(
    lichess: &Lichess,
    id: &str,
    chapter: &str,
) -> CoreResult<ChapterInput> {
    let path = if chapter.is_empty() {
        format!("/api/study/{}.pgn", path_segment(id))
    } else {
        format!(
            "/api/study/{}/{}.pgn",
            path_segment(id),
            path_segment(chapter)
        )
    };
    let query = [
        ("comments", "true".to_string()),
        ("variations", "true".to_string()),
        ("clocks", "false".to_string()),
        ("orientation", "true".to_string()),
    ];
    let text = lichess
        .client()
        .get(&path, &lichess.anonymous(UsageKind::Study))
        .query(&query)
        .accept("application/x-chess-pgn")
        .read_text(Some(STUDY_PGN_LIMIT))
        .await?;
    let pgn = split_pgn(&text)
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::new("That study has no chapter to open."))?;
    let name = tag_value(&pgn, "ChapterName")
        .or_else(|| tag_value(&pgn, "Event"))
        .unwrap_or_else(|| "Chapter 1".into());
    Ok(ChapterInput {
        name: js::slice_utf16(&name, 120).to_string(),
        pgn,
    })
}

/// Adds a game to a Lichess study as new
/// chapters, or to a new private study when `study_id` is empty. Returns the study id once
/// Lichess reports a chapter for every game: a full study answers success yet adds none.
pub async fn export_to_lichess_study(
    lichess: &Lichess,
    account: &str,
    study_id: &str,
    name: &str,
    pgn: &str,
) -> CoreResult<Reply<String>> {
    lichess
        .as_account(account, UsageKind::Study, |auth| async move {
            let mut id = study_id.to_string();
            if id.is_empty() {
                let created: Value = lichess
                    .client()
                    .post("/api/study", &auth)
                    .form(&[
                        ("name", name),
                        ("visibility", "private"),
                        ("computer", "everyone"),
                        ("explorer", "everyone"),
                        ("cloneable", "owner"),
                        ("shareable", "owner"),
                        ("chat", "member"),
                    ])
                    .read_json()
                    .await?;
                id = created
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| CoreError::new("Lichess did not create the study."))?;
            }
            let path = format!("/api/study/{}/import-pgn", path_segment(&id));
            let imported: Value = lichess
                .client()
                .post(&path, &auth)
                .form(&[("pgn", pgn), ("name", name)])
                .read_json()
                .await?;
            imported_all(&imported, split_pgn(pgn).len().max(1))?;
            Ok(id)
        })
        .await
}

/// Lichess reported a chapter for each of `games` it was sent: a full study answers success yet
/// adds none.
fn imported_all(imported: &Value, games: usize) -> CoreResult<()> {
    let added = imported
        .get("chapters")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if added == 0 {
        return Err(CoreError::new(format!(
            "Lichess added no chapters. A study holds at most {MAX_CHAPTERS} chapters."
        )));
    }
    if added < games {
        return Err(CoreError::new(format!(
            "Lichess added only {added} of {games} chapters. A study holds at most {MAX_CHAPTERS} chapters."
        )));
    }
    Ok(())
}

/// Uploads edits to a linked study. The cloud copy must still match
/// the baseline the user downloaded; otherwise nothing is written and the user is told so. Only
/// changed chapters are updated, and new chapters are appended. Returns the study's chapters
/// after the upload, once Lichess holds every chapter of the edit; an upload Lichess accepted
/// only partly fails, and uploading again resumes it.
pub async fn sync_lichess_study(
    lichess: &Lichess,
    request: &StudySyncRequest,
) -> CoreResult<Reply<Vec<ChapterInput>>> {
    let before = valid_chapters(&request.baseline)?;
    let after = valid_chapters(&request.pgn)?;
    let wanted = after.len();
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
    // An earlier upload may have stopped partway: chapters it already wrote (and chapters it
    // already appended) match the local edit rather than the baseline, and a chapter whose moves
    // were written before its tags were not holds the edit's moves with the baseline's tags.
    // Anything else changed on Lichess, and is never overwritten.
    let mut pending = vec![Pending::Chapter; before.len()];
    let mut changed_remotely = remote.len() < before.len() || remote.len() > after.len();
    if !changed_remotely {
        for (index, game) in remote.iter().enumerate() {
            match before.get(index) {
                Some(expected) if game.pgn == expected.pgn => {}
                Some(expected) => match interrupted(game, expected, &after[index]) {
                    Some(left) => pending[index] = left,
                    None => changed_remotely = true,
                },
                None if uploaded(game, &after[index], &[]) => {}
                _ => changed_remotely = true,
            }
        }
    }
    let appended = remote.len().saturating_sub(before.len());
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
    let result = lichess
        .as_account(&request.account, UsageKind::Study, |auth| async move {
            for (index, expected) in before.iter().enumerate() {
                let game = &after[index];
                if game.pgn == expected.pgn || pending[index] == Pending::Nothing {
                    continue;
                }
                let base = format!(
                    "/api/study/{}/{}",
                    path_segment(&request.study_id),
                    path_segment(&ids[index])
                );
                // Moves already written are never sent again.
                if pending[index] == Pending::Chapter {
                    let _: Value = lichess
                        .client()
                        .post(&format!("{base}/moves"), &auth)
                        .form(&[("pgn", game.pgn.as_str())])
                        .read_json()
                        .await?;
                }
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
                    .post(&format!("{base}/tags"), &auth)
                    .form(&[("pgn", tags_pgn.as_str())])
                    .read_json()
                    .await?;
            }
            for game in after.iter().skip(before.len() + appended) {
                let name = map_get(&game.headers, "ChapterName")
                    .or_else(|| map_get(&game.headers, "Event"))
                    .unwrap_or("New chapter")
                    .to_string();
                let path = format!("/api/study/{}/import-pgn", path_segment(&request.study_id));
                let imported: Value = lichess
                    .client()
                    .post(&path, &auth)
                    .form(&[("pgn", game.pgn.as_str()), ("name", name.as_str())])
                    .read_json()
                    .await?;
                imported_all(&imported, 1)?;
            }
            Ok(())
        })
        .await?;
    if let Reply::NeedsReconnect = result {
        return Ok(Reply::NeedsReconnect);
    }
    let chapters =
        match lichess_study_chapters(lichess, &request.account, &request.study_id).await? {
            Reply::Done(chapters) => chapters,
            Reply::NeedsReconnect => return Ok(Reply::NeedsReconnect),
        };
    if chapters.len() < wanted {
        return Err(CoreError::new(format!(
            "Lichess holds only {} of {} chapters. Upload again to finish; your offline edits are kept.",
            chapters.len(),
            wanted
        )));
    }
    Ok(Reply::Done(chapters))
}

/// The movetext of a chapter's PGN: everything after its tag pairs.
fn movetext(pgn: &str) -> &str {
    let mut rest = pgn.trim_start();
    while rest.starts_with('[') {
        rest = rest
            .split_once('\n')
            .map_or("", |(_, tail)| tail)
            .trim_start();
    }
    rest.trim_end()
}

/// What an upload still has to write to one existing chapter.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    /// Its moves, then its tags.
    Chapter,
    /// Its tags: the moves were written before the upload stopped.
    Tags,
    Nothing,
}

/// Lichess holds `tags` of `chapter` (the chapter identity in `[Site]` stays Lichess's), and
/// none of the `removed` ones.
fn has_tags(remote: &Chapter, chapter: &Chapter, removed: &[&str]) -> bool {
    chapter
        .headers
        .iter()
        .filter(|(key, _)| key != "Site")
        .all(|(key, value)| map_get(&remote.headers, key) == Some(value.as_str()))
        && removed
            .iter()
            .all(|key| map_get(&remote.headers, key).is_none_or(str::is_empty))
}

/// Lichess already holds the local `chapter`: the same moves, every tag the upload sets, and
/// none it removes.
fn uploaded(remote: &Chapter, chapter: &Chapter, removed: &[&str]) -> bool {
    movetext(&remote.pgn) == movetext(&chapter.pgn) && has_tags(remote, chapter, removed)
}

/// A chapter an earlier upload of `edit` over `baseline` stopped partway through: what is left
/// to write, or `None` when Lichess holds something this upload never wrote.
fn interrupted(remote: &Chapter, baseline: &Chapter, edit: &Chapter) -> Option<Pending> {
    let removed: Vec<&str> = baseline
        .headers
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| *key != "Site" && map_get(&edit.headers, key).is_none())
        .collect();
    if uploaded(remote, edit, &removed) {
        Some(Pending::Nothing)
    } else if movetext(&remote.pgn) == movetext(&edit.pgn) && has_tags(remote, baseline, &[]) {
        Some(Pending::Tags)
    } else {
        None
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
        let ids_ok = super::is_lichess_id(study) && super::is_lichess_id(chapter);
        if ids_ok && tail_ok {
            return Some((study.to_string(), chapter.to_string()));
        }
    }
    None
}

/// A chapter with its headers (first key order, last value).
#[derive(Clone, Debug, PartialEq)]
struct Chapter {
    headers: Vec<(String, String)>,
    pgn: String,
}

/// The games of a PGN, each replayable from its start and with legal moves.
fn valid_chapters(pgn: &str) -> CoreResult<Vec<Chapter>> {
    let games = position::pgn_games(pgn);
    if games.is_empty() || games.len() > MAX_CHAPTERS {
        return Err(CoreError::new("Use up to 64 valid chapters."));
    }
    let mut chapters = Vec::with_capacity(games.len());
    for game in games {
        match game.problem {
            Some("start") => return Err(CoreError::new("Invalid chapter starting position.")),
            Some("move") => return Err(CoreError::new("The chapter contains an illegal move.")),
            _ => {}
        }
        let mut headers = Vec::new();
        for (key, value) in &game.headers {
            map_set(&mut headers, key, value);
        }
        chapters.push(Chapter {
            headers,
            pgn: game.pgn,
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
        // Both ids must be ids, and nothing but a query or fragment may follow.
        for site in [
            "https://lichess.org/study/Study-01/Chapter1",
            "https://lichess.org/study/Study001/Chapter-",
            "https://lichess.org/study/Study001/Chapter12",
        ] {
            assert_eq!(study_chapter_id(site), None, "{site}");
        }
    }

    #[test]
    fn lists_studies_with_the_contract_field_names() {
        let study = study_metadata(
            &serde_json::json!({ "id": "Study001", "name": "Cloud", "createdAt": 1, "updatedAt": 2 }),
        )
        .expect("a valid study record");
        assert_eq!(
            serde_json::to_value(&study).expect("serializable"),
            serde_json::json!({ "id": "Study001", "name": "Cloud", "updatedAt": 2 })
        );
        let undated = study_metadata(&serde_json::json!({ "id": "Study002", "name": "Undated" }))
            .expect("a record without timestamps");
        assert_eq!(undated.updated_at, 0);
    }
}
