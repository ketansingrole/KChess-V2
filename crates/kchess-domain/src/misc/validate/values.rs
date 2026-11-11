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

pub(super) const MAX_SAFE: u64 = MAX_SAFE_INTEGER as u64;

pub(super) fn assert_review_key(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid review."))?;
    matches(&text, patterns::review_key, Some("Invalid review."), "", "")?;
    Ok(text)
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
