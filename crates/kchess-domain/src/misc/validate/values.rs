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

/// A position: a full FEN, at most 100 characters.
pub(super) fn assert_fen(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid position."))?;
    fen(
        &text,
        Some("Invalid position."),
        Some("Invalid position."),
        "",
    )?;
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

pub(super) fn assert_uci(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid move."))?;
    uci(&text)?;
    Ok(text)
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

pub(super) fn assert_usernames(value: J) -> Checked<Vec<String>> {
    usernames(value, None, 50)
}

pub(super) fn assert_friend_list(value: J) -> Checked<Vec<String>> {
    usernames(value, Some("Choose at least one player."), 1000)
}

pub(super) fn assert_action(value: J) -> Checked<String> {
    picked(value, ONLINE_ACTIONS, Some("Invalid game action."))
}

pub(super) fn assert_relation_action(value: J) -> Checked<String> {
    picked(value, RELATION_ACTIONS, Some("Invalid action."))
}

pub(super) fn assert_chat_room(value: J) -> Checked<String> {
    picked(value, CHAT_ROOMS, Some("Invalid chat room."))
}

/// A message to send, trimmed, of 1 to `max` characters.
pub(super) fn message_text(value: J, max: usize, too_long: &'static str) -> Checked<String> {
    let text: String = decode(value, always!("Invalid message."))?;
    let text = js::trim(&text);
    min_chars(text, 1, Some("Type a message first."), "")?;
    max_chars(text, max, Some(too_long), "")?;
    Ok(text.to_owned())
}

pub(super) fn assert_chat_text(value: J) -> Checked<String> {
    message_text(value, 140, "Chat messages are limited to 140 characters.")
}

pub(super) fn assert_message_text(value: J) -> Checked<String> {
    message_text(value, 8000, "Messages are limited to 8,000 characters.")
}

pub(super) fn assert_decline_reason(value: J) -> Checked<String> {
    picked(value, DECLINE_REASONS, Some("Invalid reason."))
}

pub(super) fn assert_tournament_system(value: J) -> Checked<String> {
    picked(value, TOURNAMENT_SYSTEMS, Some("Invalid tournament."))
}

pub(super) fn assert_tournament_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid tournament."))?;
    lichess_id(&text, "Invalid tournament.")?;
    Ok(text)
}

/// `assertTournamentPassword`: nothing or an empty password means none.
pub(super) fn assert_tournament_password(value: J) -> Checked<Option<String>> {
    if matches!(&value, J::Undef) {
        return Ok(None);
    }
    let text: String = decode(value, always!("Invalid password."))?;
    if text.is_empty() {
        return Ok(None);
    }
    max_chars(&text, 100, Some("Invalid password."), "")?;
    Ok(Some(text))
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

/// The phrases speech recognition listens for (`voiceStart`): at most 4,096 of at most 200
/// characters, as the `kchess-voice` helper accepts.
pub(super) fn assert_voice_grammar(value: J) -> Checked<Vec<String>> {
    const INVALID: &str = "Invalid voice grammar.";
    let phrases: Vec<String> = decode_or(value, Some(INVALID))?;
    within(phrases.len(), 1, 4096, Some(INVALID), "")?;
    for phrase in &phrases {
        max_chars(phrase, 200, Some(INVALID), "")?;
    }
    Ok(phrases)
}

/// The microphone's sample rate in Hz (`voiceStart`).
pub(super) fn assert_sample_rate(value: J) -> Checked<f64> {
    const INVALID: &str = "Invalid sample rate.";
    let rate: f64 = decode_or(value, Some(INVALID))?;
    finite(rate, Some(INVALID), "")?;
    within(rate, 8_000.0, 192_000.0, Some(INVALID), "")?;
    Ok(rate)
}

/// One chunk of base64 audio (`voiceAccept`), no larger than the helper's 1 MiB request limit.
pub(super) fn assert_voice_audio(value: J) -> Checked<String> {
    const INVALID: &str = "Invalid audio.";
    let audio: String = decode_or(value, Some(INVALID))?;
    let base64 = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=');
    ensure(
        audio.len().is_multiple_of(4)
            && audio.len() <= (1 << 20) / 3 * 4
            && audio.bytes().all(base64),
        Some(INVALID),
        "",
        || "base64 audio".into(),
    )?;
    Ok(audio)
}

/// Bounded text for Lichess broadcast search, trimmed; nothing is no search.
pub(super) fn assert_broadcast_query(value: J) -> Checked<Option<String>> {
    let text: Option<String> = decode(value, plain)?;
    text.map(|text| {
        let text = js::trim(&text);
        max_chars(text, 100, None, "")?;
        Ok(text.to_owned())
    })
    .transpose()
}

/// `CORE_CONTRACTS` `lines` (`cloudEval`): an integer from 1 to 5.
pub(super) fn assert_lines(value: J) -> Checked<u64> {
    let n: u64 = decode(value, plain)?;
    within(n, 1, 5, None, "")?;
    Ok(n)
}

/// `CORE_CONTRACTS` `optionalPage` (`tournament`): nothing, or a page from 1 to 200.
pub(super) fn assert_optional_page(value: J) -> Checked<Option<u64>> {
    let page: Option<u64> = decode(value, plain)?;
    if let Some(n) = page {
        within(n, 1, 200, None, "")?;
    }
    Ok(page)
}

/// `stopWatching`'s session: the number a watch's `watch` or `watchBroadcast` answered.
pub(super) fn assert_watch_session(value: J) -> Checked<u64> {
    let session: u64 = decode(value, plain)?;
    within(session, 1, u64::from(u32::MAX), None, "")?;
    Ok(session)
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
