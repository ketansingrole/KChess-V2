//! The arguments of the Lichess studies, games and opening pages: ids, chapter names, PGN text
//! and lines of moves.

use serde::{Deserialize, Serialize};

use kchess_domain::js;
use kchess_domain::misc::constants::MAX_MOVES;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;

use super::*;
use crate::types::StudySyncRequest;

/// The id of a Lichess study, chapter or broadcast.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct LichessId(pub String);

impl Validate for LichessId {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid id.";
        let text: String = decode(value, |_| Some(INVALID))?;
        matches(&text, patterns::lichess_id, Some(INVALID), "", "")?;
        Ok(LichessId(text))
    }
}

/// A Lichess study's id, or empty for a new study (or a study's first chapter).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct StudyId(pub String);

impl Validate for StudyId {
    fn validate(value: J) -> Result<Self, Refusal> {
        if matches!(&value, J::Str(text) if text.is_empty()) {
            return Ok(StudyId::default());
        }
        LichessId::validate(value).map(|LichessId(id)| StudyId(id))
    }
}

/// A Lichess game's id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct GameId(pub String);

const GAME_ID: &str = "Invalid game id.";

impl Validate for GameId {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, |_| Some(GAME_ID))?;
        matches(&text, patterns::game_id, Some(GAME_ID), "", "")?;
        Ok(GameId(text))
    }
}

/// A chapter's name, trimmed: 1 to 100 characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct ChapterName(pub String);

impl Validate for ChapterName {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, plain)?;
        let text = js::trim(&text);
        min_chars(text, 1, None, "")?;
        max_chars(text, 100, None, "")?;
        Ok(ChapterName(text.to_owned()))
    }
}

/// PGN text to upload: 1 to 500,000 characters, not all blank.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PgnText(pub String);

impl Validate for PgnText {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, plain)?;
        min_chars(&text, 1, None, "")?;
        max_chars(&text, 500_000, None, "")?;
        ensure(!text.trim().is_empty(), None, "", || "some text".into())?;
        Ok(PgnText(text))
    }
}

/// One UCI move.
pub fn uci(text: &str) -> Result<(), Refusal> {
    matches(text, patterns::uci_move, Some("Invalid move."), "", "")
}

/// The messages of a move list as sent: the list's own, and `Invalid move.` for an item.
pub fn move_list_messages(keys: &[&str]) -> Msg {
    match keys {
        [] => Some("Invalid move list."),
        _ => Some("Invalid move."),
    }
}

/// UCI moves, no more than a game holds.
pub fn move_list(moves: &[String]) -> Result<(), Refusal> {
    for played in moves {
        uci(played)?;
    }
    max_items(moves, MAX_MOVES, Some("Invalid move list."), "")
}

/// A line of UCI moves.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct Moves(pub Vec<String>);

impl Validate for Moves {
    fn validate(value: J) -> Result<Self, Refusal> {
        let moves: Vec<String> = decode(value, move_list_messages)?;
        move_list(&moves)?;
        Ok(Moves(moves))
    }
}

const STUDY_SYNC: &str = "Invalid study synchronization.";
const STUDY_TEXT: &str = "The study is empty or too large to upload.";
const USERNAME: &str = "Enter a valid Lichess username.";

/// The upload as sent: every field checked for its own message.
#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase")]
struct SentSync {
    account: Option<String>,
    study_id: Option<String>,
    baseline: Option<String>,
    pgn: Option<String>,
}

impl Validate for StudySyncRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        if !value.is_object() {
            return Err(STUDY_SYNC.into());
        }
        let sent: SentSync = decode(value, |keys| match keys {
            ["account"] => Some(USERNAME),
            ["studyId"] => Some("Invalid id."),
            ["baseline" | "pgn"] => Some(STUDY_TEXT),
            _ => Some(STUDY_SYNC),
        })?;
        let account = sent.account.as_deref().ok_or(USERNAME)?;
        let account = js::trim(account);
        matches(account, patterns::username, Some(USERNAME), "", "")?;
        let study_id = sent.study_id.ok_or("Invalid id.")?;
        matches(&study_id, patterns::lichess_id, Some("Invalid id."), "", "")?;
        let usable = |text: &Option<String>| {
            text.as_deref()
                .is_some_and(|t| !js::trim(t).is_empty() && js::utf16_len(t) <= 500_000)
        };
        ensure(
            usable(&sent.baseline) && usable(&sent.pgn),
            Some(STUDY_TEXT),
            "",
            String::new,
        )?;
        Ok(StudySyncRequest {
            account: account.to_owned(),
            study_id,
            baseline: sent.baseline.unwrap_or_default(),
            pgn: sent.pgn.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_study_id_is_a_new_study() {
        assert_eq!(
            check::<StudyId>(J::Str(String::new())),
            Ok(StudyId::default())
        );
        assert_eq!(
            check::<StudyId>(J::Str("not an id!".into())),
            Err("Invalid id.".into())
        );
    }

    #[test]
    fn a_chapter_name_is_trimmed() {
        assert_eq!(
            check::<ChapterName>(J::Str("  Rook endings ".into())),
            Ok(ChapterName("Rook endings".into()))
        );
        assert!(check::<ChapterName>(J::Str("   ".into())).is_err());
        assert!(check::<PgnText>(J::Str("  ".into())).is_err());
    }
}
