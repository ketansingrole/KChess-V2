//! Validators of single values: names, ids, moves, choices and bounded numbers and texts.

use super::*;

pub fn assert_username(value: J) -> Checked<String> {
    let text: String = decode(value, always!(USERNAME))?;
    username(&text, "")
}

pub(super) fn assert_game_account(value: J) -> Checked<String> {
    let text: String = decode(value, always!(USERNAME))?;
    game_account(&text, "")
}

pub(super) fn assert_chesscom_username(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Enter a valid Chess.com username."))?;
    chesscom_username(&text)
}

pub(super) fn assert_game_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid game id."))?;
    game_id(&text)?;
    Ok(text)
}

pub(super) fn assert_game_ids(value: J) -> Checked<Vec<String>> {
    let ids: Vec<String> = decode(value, |keys| match keys {
        [] => Some("Invalid game list."),
        _ => Some("Invalid game id."),
    })?;
    each(&ids, |id| game_id(id))?;
    max_items(&ids, 100, Some("Too many games."), "")?;
    Ok(ids)
}

pub fn assert_moves(value: J) -> Checked<Vec<String>> {
    let moves: Vec<String> = decode(value, |keys| {
        move_list_messages(keys, Some("Invalid move list."))
    })?;
    move_list(&moves)?;
    Ok(moves)
}

/// Decodes `value` as `T`; any rejection is `msg`, or the decoder's own text without one.
pub(super) fn decode_or<T: DeserializeOwned>(value: J, msg: Msg) -> Checked<T> {
    decode(value, plain).map_err(|error| msg.map_or(error, str::to_owned))
}

/// A string from `options`; any rejection is `msg`.
pub(super) fn picked(value: J, options: &[&str], msg: Msg) -> Checked<String> {
    let text: String = decode_or(value, msg)?;
    one_of(&text, options, msg, "")?;
    Ok(text)
}

pub(super) fn assert_level(value: J) -> Checked<String> {
    picked(value, ENGINE_LEVELS, Some("Invalid engine level."))
}

/// A list of usernames (trimmed), at most `max`; `min_msg` when there must be at least one.
pub(super) fn usernames(value: J, min_msg: Msg, max: usize) -> Checked<Vec<String>> {
    let names: Vec<String> = decode(value, |keys| match keys {
        [] => Some("Invalid users."),
        _ => Some(USERNAME),
    })?;
    let names: Vec<String> = names
        .iter()
        .map(|name| username(name, ""))
        .collect::<Checked<_>>()?;
    if min_msg.is_some() {
        min_items(&names, 1, min_msg, "")?;
    }
    max_items(&names, max, Some("Invalid users."), "")?;
    Ok(names)
}

pub(super) fn assert_friend_list(value: J) -> Checked<Vec<String>> {
    usernames(value, Some("Choose at least one player."), 1000)
}

pub(super) fn assert_relation_action(value: J) -> Checked<String> {
    picked(value, RELATION_ACTIONS, Some("Invalid action."))
}

/// A message to send, trimmed, of 1 to `max` characters.
pub(super) fn message_text(value: J, max: usize, too_long: &'static str) -> Checked<String> {
    let text: String = decode(value, always!("Invalid message."))?;
    let text = js::trim(&text);
    min_chars(text, 1, Some("Type a message first."), "")?;
    max_chars(text, max, Some(too_long), "")?;
    Ok(text.to_owned())
}

pub(super) fn assert_message_text(value: J) -> Checked<String> {
    message_text(value, 8000, "Messages are limited to 8,000 characters.")
}

/// A variant KChess plays (`VARIANTS`).
pub(super) fn assert_variant(value: J) -> Checked<String> {
    picked(value, VARIANTS, Some("Invalid variant."))
}

pub(super) fn assert_perf_type(value: J) -> Checked<String> {
    picked(value, PERF_TYPES, Some("Invalid rating category."))
}

/// A puzzle's id, as a lichess.org/training link names it.
pub(super) fn assert_puzzle_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid puzzle."))?;
    matches(&text, patterns::puzzle_id, Some("Invalid puzzle."), "", "")?;
    Ok(text)
}

pub(super) fn assert_lichess_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid id."))?;
    lichess_id(&text, "Invalid id.")?;
    Ok(text)
}

/// `optionalAccountSchema`: a connected account, or empty for none.
pub(super) fn assert_optional_account(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid account."))?;
    if text.is_empty() {
        return Ok(text);
    }
    username(&text, "")
}

/// An integer from `min` to `max`; any rejection is `msg`.
pub(super) fn integer(value: J, min: u64, max: u64, msg: &'static str) -> Checked<u64> {
    let n: u64 = decode_or(value, Some(msg))?;
    within(n, min, max, Some(msg), "")?;
    Ok(n)
}

pub(super) const MAX_SAFE: u64 = MAX_SAFE_INTEGER as u64;

pub(super) fn assert_days(value: J) -> Checked<u64> {
    integer(value, 1, 365, "Invalid number of days.")
}

pub(super) fn assert_activity_max(value: J) -> Checked<u64> {
    integer(value, 1, 200, "Invalid number of puzzles.")
}

pub(super) fn assert_run_kind(value: J) -> Checked<String> {
    picked(value, RUN_KINDS, Some("Invalid kind of run."))
}

pub(super) fn assert_review_key(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid review."))?;
    matches(&text, patterns::review_key, Some("Invalid review."), "", "")?;
    Ok(text)
}

/// A model the account listed, or null for its first.
pub(super) fn assert_ai_model(value: J) -> Checked<Option<String>> {
    let model: Option<String> = decode(value, always!("Invalid model."))?;
    if model
        .as_deref()
        .is_some_and(|model| model.is_empty() || model.len() > 100)
    {
        return Err("Invalid model.".into());
    }
    Ok(model)
}

pub(super) fn assert_ai_effort(value: J) -> Checked<String> {
    picked(value, AI_EFFORTS, Some("Invalid effort."))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AiRequest {
    kind: String,
    fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    played: Option<String>,
    #[serde(default)]
    moves: Vec<String>,
    #[serde(default)]
    best: Vec<String>,
    #[serde(default)]
    refutation: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mate_before: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mate_after: Option<f64>,
}

pub(super) fn assert_ai_request(value: J) -> Checked<AiRequest> {
    const INVALID: &str = "That position cannot be explained.";
    let request: AiRequest = decode(value, always!(INVALID))?;
    let lines = [&request.moves, &request.best, &request.refutation];
    let long = lines.iter().any(|line| line.len() > 64);
    if !["move", "nudge", "solution", "wrong"].contains(&request.kind.as_str())
        || request.fen.len() > 100
        || long
        || request
            .played
            .as_ref()
            .is_some_and(|played| played.len() > 5)
    {
        return Err(INVALID.into());
    }
    Ok(request)
}

pub(super) fn assert_voice_id(value: J) -> Checked<u64> {
    integer(value, 1, MAX_SAFE, VOICE)
}

pub(super) fn assert_voice_limit(value: J) -> Checked<u64> {
    integer(value, 1, 5000, "Invalid number of entries.")
}

/// `CORE_CONTRACTS` `lines` (`cloudEval`): an integer from 1 to 5.
pub(super) fn assert_lines(value: J) -> Checked<u64> {
    let n: u64 = decode(value, plain)?;
    within(n, 1, 5, None, "")?;
    Ok(n)
}

/// `CORE_CONTRACTS` `boolean` (`answerMistake`): a boolean.
pub(super) fn assert_boolean(value: J) -> Checked<bool> {
    decode(value, plain)
}

/// `CORE_CONTRACTS` `chapterName`: a trimmed name of 1 to 100 characters.
pub(super) fn assert_chapter_name(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    let text = js::trim(&text);
    min_chars(text, 1, None, "")?;
    max_chars(text, 100, None, "")?;
    Ok(text.to_owned())
}

/// `CORE_CONTRACTS` `pgn` (`exportToLichessStudy`): 1 to 500,000 characters with some content.
pub(super) fn assert_pgn_text(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    min_chars(&text, 1, None, "")?;
    max_chars(&text, 500_000, None, "")?;
    ensure(!text.trim().is_empty(), None, "", || "some text".into())?;
    Ok(text)
}

/// `CORE_CONTRACTS` `sessionObject` (`saveSession`): any object; the session is checked after it.
pub(super) fn assert_session_object(value: J) -> Checked<Value> {
    ensure(value.is_object(), None, "", || "an object".into())?;
    Ok(Value::Object(serde_json::Map::new()))
}

/// `CORE_CONTRACTS` `studyId` (`exportToLichessStudy`): nothing (a new study) or a Lichess id.
pub(super) fn assert_study_id(value: J) -> Checked<String> {
    if matches!(&value, J::Str(text) if text.is_empty()) {
        return Ok(String::new());
    }
    assert_lichess_id(value)
}
