//! Validators of single values: names, ids, moves, choices and bounded numbers and texts.

use super::*;

pub fn assert_username(value: J) -> Checked<String> {
    let text: String = decode(value, always!(USERNAME))?;
    username(&text, "")
}

pub(super) fn assert_game_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid game id."))?;
    game_id(&text)?;
    Ok(text)
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

/// A variant KChess plays (`VARIANTS`).
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

pub(super) fn assert_voice_id(value: J) -> Checked<u64> {
    integer(value, 1, MAX_SAFE, VOICE)
}

pub(super) fn assert_voice_limit(value: J) -> Checked<u64> {
    integer(value, 1, 5000, "Invalid number of entries.")
}

/// `CORE_CONTRACTS` `boolean` (`answerMistake`): a boolean.
pub(super) fn assert_boolean(value: J) -> Checked<bool> {
    decode(value, plain)
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
