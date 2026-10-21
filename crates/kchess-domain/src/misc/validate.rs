//! The `assert*` validators of `crates/kchess-wasm/js/validate.ts`: each decodes an untrusted IPC
//! value into a typed struct, checks it, and returns the normalized value or a message.
//!
//! A value of the wrong type takes the message its validator names for that place (the field, or
//! the object for a missing property); without one, the decoder's own text, prefixed by the path
//! (`speeds[0]: unknown variant …`). Type errors come first, in the order the input lists its
//! properties; the checks after them run in field order.

use serde::de::{self, DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::constants::*;
use super::patterns;
use super::value::{self, J};
use crate::{js, replay};

pub type Validator = fn(Vec<J>) -> Result<Value, String>;

/// A custom message, as the TypeScript source wrote it.
type Msg = Option<&'static str>;

type Checked<T> = Result<T, String>;

/* ── Decoding and checks ── */

/// The first argument, moved out of the list (`undefined` when it is absent).
fn first(args: Vec<J>) -> J {
    args.into_iter().next().unwrap_or(J::Undef)
}

/// Decodes `value` as `T`. A rejection takes the message `messages` names for the keys of the
/// rejected value's path (`[]` for a list position), else the decoder's own text.
fn decode<T: DeserializeOwned>(value: J, messages: fn(&[&str]) -> Msg) -> Checked<T> {
    value::decode(value).map_err(|error| match messages(&error.keys()) {
        Some(message) => message.to_owned(),
        None => error.to_string(),
    })
}

/// Messages for a validator that names one message for every rejection.
macro_rules! always {
    ($msg:expr) => {
        (|_: &[&str]| Some($msg)) as fn(&[&str]) -> Msg
    };
}

/// No custom messages.
fn plain(_: &[&str]) -> Msg {
    None
}

/// The normalized value, as JSON (integral numbers as integers).
fn out<T: Serialize>(value: &T) -> Checked<Value> {
    serde_json::to_value(value)
        .map(value::integral)
        .map_err(|error| error.to_string())
}

/// `msg`, or a description of what the value at `path` should have been.
fn reject(msg: Msg, path: &str, expected: &str) -> String {
    match msg {
        Some(message) => message.to_owned(),
        None if path.is_empty() => format!("invalid value, expected {expected}"),
        None => format!("{path}: invalid value, expected {expected}"),
    }
}

fn ensure(ok: bool, msg: Msg, path: &str, expected: impl FnOnce() -> String) -> Checked<()> {
    if ok {
        Ok(())
    } else {
        Err(reject(msg, path, &expected()))
    }
}

/// At most `max` UTF-16 units.
fn max_chars(text: &str, max: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(js::utf16_len(text) <= max, msg, path, || {
        format!("at most {max} characters")
    })
}

/// At least `min` UTF-16 units.
fn min_chars(text: &str, min: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(js::utf16_len(text) >= min, msg, path, || match min {
        1 => "at least 1 character".into(),
        _ => format!("at least {min} characters"),
    })
}

fn max_items<T>(items: &[T], max: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(items.len() <= max, msg, path, || {
        format!("at most {max} items")
    })
}

fn min_items<T>(items: &[T], min: usize, msg: Msg, path: &str) -> Checked<()> {
    ensure(items.len() >= min, msg, path, || match min {
        1 => "at least 1 item".into(),
        _ => format!("at least {min} items"),
    })
}

fn within<N: PartialOrd + std::fmt::Display>(
    n: N,
    min: N,
    max: N,
    msg: Msg,
    path: &str,
) -> Checked<()> {
    let ok = n >= min && n <= max;
    ensure(ok, msg, path, || format!("a number from {min} to {max}"))
}

fn finite(n: f64, msg: Msg, path: &str) -> Checked<()> {
    ensure(n.is_finite(), msg, path, || "a finite number".into())
}

fn one_of(text: &str, options: &[&str], msg: Msg, path: &str) -> Checked<()> {
    ensure(options.contains(&text), msg, path, || {
        format!("one of {}", options.join(", "))
    })
}

fn one_of_numbers(n: f64, options: &[f64], msg: Msg, path: &str) -> Checked<()> {
    ensure(options.contains(&n), msg, path, || {
        let listed: Vec<String> = options.iter().map(f64::to_string).collect();
        format!("one of {}", listed.join(", "))
    })
}

fn matches(text: &str, test: fn(&str) -> bool, msg: Msg, path: &str, what: &str) -> Checked<()> {
    ensure(test(text), msg, path, || what.to_owned())
}

/// Each item checked by `check`, in order.
fn each<T>(items: &[T], check: impl Fn(&T) -> Checked<()>) -> Checked<()> {
    items.iter().try_for_each(check)
}

/* ── Shared fields ── */

const USERNAME: &str = "Enter a valid Lichess username.";
const NOTIFY: &str = "Invalid notification setting.";
const VOICE: &str = "Invalid voice entry.";
const ATTEMPT: &str = "Invalid puzzle attempt.";

/// A Lichess username, trimmed.
fn username(text: &str, path: &str) -> Checked<String> {
    let trimmed = js::trim(text);
    matches(trimmed, patterns::username, Some(USERNAME), path, "")?;
    Ok(trimmed.to_owned())
}

fn game_id(text: &str) -> Checked<()> {
    matches(text, patterns::game_id, Some("Invalid game id."), "", "")
}

fn uci(text: &str) -> Checked<()> {
    matches(text, patterns::uci_move, Some("Invalid move."), "", "")
}

/// A move list: each a UCI move, and no more than a game holds.
fn move_list(moves: &[String]) -> Checked<()> {
    each(moves, |m| uci(m))?;
    max_items(moves, MAX_MOVES, Some("Invalid move list."), "")
}

fn angle(text: &str) -> Checked<()> {
    matches(
        text,
        patterns::puzzle_angle,
        Some("Invalid puzzle theme."),
        "",
        "",
    )
}

fn lichess_id(text: &str, msg: &'static str) -> Checked<()> {
    matches(text, patterns::lichess_id, Some(msg), "", "")
}

/// A position: at most 100 characters (`length` when longer), then the FEN shape (`msg`).
fn fen(text: &str, length: Msg, msg: Msg, path: &str) -> Checked<()> {
    max_chars(text, 100, length, path)?;
    matches(text, patterns::fen, msg, path, "a FEN position")
}

fn local_day(text: &str, msg: &'static str) -> Checked<()> {
    matches(text, patterns::local_day, Some(msg), "", "")
}

/// Messages of a move list, where the list itself has `list` and each move has its own.
fn move_list_messages(keys: &[&str], list: Msg) -> Msg {
    match keys {
        [] => list,
        _ => Some("Invalid move."),
    }
}

/* ── Single values ── */

fn assert_username(value: J) -> Checked<String> {
    let text: String = decode(value, always!(USERNAME))?;
    username(&text, "")
}

fn assert_game_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid game id."))?;
    game_id(&text)?;
    Ok(text)
}

fn assert_game_ids(value: J) -> Checked<Vec<String>> {
    let ids: Vec<String> = decode(value, |keys| match keys {
        [] => Some("Invalid game list."),
        _ => Some("Invalid game id."),
    })?;
    each(&ids, |id| game_id(id))?;
    max_items(&ids, 100, Some("Too many games."), "")?;
    Ok(ids)
}

fn assert_uci(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid move."))?;
    uci(&text)?;
    Ok(text)
}

fn assert_moves(value: J) -> Checked<Vec<String>> {
    let moves: Vec<String> = decode(value, |keys| {
        move_list_messages(keys, Some("Invalid move list."))
    })?;
    move_list(&moves)?;
    Ok(moves)
}

/// Decodes `value` as `T`; any rejection is `msg`, or the decoder's own text without one.
fn decode_or<T: DeserializeOwned>(value: J, msg: Msg) -> Checked<T> {
    decode(value, plain).map_err(|error| msg.map_or(error, str::to_owned))
}

/// A string from `options`; any rejection is `msg`.
fn picked(value: J, options: &[&str], msg: Msg) -> Checked<String> {
    let text: String = decode_or(value, msg)?;
    one_of(&text, options, msg, "")?;
    Ok(text)
}

fn assert_level(value: J) -> Checked<String> {
    picked(value, ENGINE_LEVELS, Some("Invalid engine level."))
}

/// A list of usernames (trimmed), at most `max`; `min_msg` when there must be at least one.
fn usernames(value: J, min_msg: Msg, max: usize) -> Checked<Vec<String>> {
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

fn assert_usernames(value: J) -> Checked<Vec<String>> {
    usernames(value, None, 50)
}

fn assert_friend_list(value: J) -> Checked<Vec<String>> {
    usernames(value, Some("Choose at least one player."), 1000)
}

fn assert_action(value: J) -> Checked<String> {
    picked(value, ONLINE_ACTIONS, Some("Invalid game action."))
}

fn assert_chat_room(value: J) -> Checked<String> {
    picked(value, CHAT_ROOMS, Some("Invalid chat room."))
}

/// A message to send, trimmed, of 1 to `max` characters.
fn message_text(value: J, max: usize, too_long: &'static str) -> Checked<String> {
    let text: String = decode(value, always!("Invalid message."))?;
    let text = js::trim(&text);
    min_chars(text, 1, Some("Type a message first."), "")?;
    max_chars(text, max, Some(too_long), "")?;
    Ok(text.to_owned())
}

fn assert_chat_text(value: J) -> Checked<String> {
    message_text(value, 140, "Chat messages are limited to 140 characters.")
}

fn assert_message_text(value: J) -> Checked<String> {
    message_text(value, 8000, "Messages are limited to 8,000 characters.")
}

fn assert_decline_reason(value: J) -> Checked<String> {
    picked(value, DECLINE_REASONS, Some("Invalid reason."))
}

fn assert_tournament_system(value: J) -> Checked<String> {
    picked(value, TOURNAMENT_SYSTEMS, Some("Invalid tournament."))
}

fn assert_tournament_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid tournament."))?;
    lichess_id(&text, "Invalid tournament.")?;
    Ok(text)
}

/// `assertTournamentPassword`: nothing or an empty password means none.
fn assert_tournament_password(value: J) -> Checked<Option<String>> {
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

fn assert_perf_type(value: J) -> Checked<String> {
    picked(value, PERF_TYPES, Some("Invalid rating category."))
}

fn assert_lichess_id(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid id."))?;
    lichess_id(&text, "Invalid id.")?;
    Ok(text)
}

/// `optionalAccountSchema`: a connected account, or empty for none.
fn assert_optional_account(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid account."))?;
    if text.is_empty() {
        return Ok(text);
    }
    username(&text, "")
}

/// An integer from `min` to `max`; any rejection is `msg`.
fn integer(value: J, min: u64, max: u64, msg: &'static str) -> Checked<u64> {
    let n: u64 = decode_or(value, Some(msg))?;
    within(n, min, max, Some(msg), "")?;
    Ok(n)
}

const MAX_SAFE: u64 = MAX_SAFE_INTEGER as u64;

fn assert_days(value: J) -> Checked<u64> {
    integer(value, 1, 365, "Invalid number of days.")
}

fn assert_activity_max(value: J) -> Checked<u64> {
    integer(value, 1, 200, "Invalid number of puzzles.")
}

fn assert_run_kind(value: J) -> Checked<String> {
    picked(value, RUN_KINDS, Some("Invalid kind of run."))
}

fn assert_review_key(value: J) -> Checked<String> {
    let text: String = decode(value, always!("Invalid review."))?;
    matches(&text, patterns::review_key, Some("Invalid review."), "", "")?;
    Ok(text)
}

fn assert_voice_id(value: J) -> Checked<u64> {
    integer(value, 1, MAX_SAFE, VOICE)
}

fn assert_voice_limit(value: J) -> Checked<u64> {
    integer(value, 1, 5000, "Invalid number of entries.")
}

/// The phrases speech recognition listens for (`voiceStart`): at most 4,096 of at most 200
/// characters, as the `kchess-voice` helper accepts.
fn assert_voice_grammar(value: J) -> Checked<Vec<String>> {
    const INVALID: &str = "Invalid voice grammar.";
    let phrases: Vec<String> = decode_or(value, Some(INVALID))?;
    within(phrases.len(), 1, 4096, Some(INVALID), "")?;
    for phrase in &phrases {
        max_chars(phrase, 200, Some(INVALID), "")?;
    }
    Ok(phrases)
}

/// The microphone's sample rate in Hz (`voiceStart`).
fn assert_sample_rate(value: J) -> Checked<f64> {
    const INVALID: &str = "Invalid sample rate.";
    let rate: f64 = decode_or(value, Some(INVALID))?;
    finite(rate, Some(INVALID), "")?;
    within(rate, 8_000.0, 192_000.0, Some(INVALID), "")?;
    Ok(rate)
}

/// One chunk of base64 audio (`voiceAccept`), no larger than the helper's 1 MiB request limit.
fn assert_voice_audio(value: J) -> Checked<String> {
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
fn assert_broadcast_query(value: J) -> Checked<Option<String>> {
    let text: Option<String> = decode(value, plain)?;
    text.map(|text| {
        let text = js::trim(&text);
        max_chars(text, 100, None, "")?;
        Ok(text.to_owned())
    })
    .transpose()
}

/// `CORE_CONTRACTS` `lines` (`cloudEval`): an integer from 1 to 5.
fn assert_lines(value: J) -> Checked<u64> {
    let n: u64 = decode(value, plain)?;
    within(n, 1, 5, None, "")?;
    Ok(n)
}

/// `CORE_CONTRACTS` `lookupKind`: one of the position lookups.
fn assert_lookup_kind(value: J) -> Checked<String> {
    picked(value, &["opening", "masters", "player", "tablebase"], None)
}

/// `CORE_CONTRACTS` `optionalPage` (`tournament`): nothing, or a page from 1 to 200.
fn assert_optional_page(value: J) -> Checked<Option<u64>> {
    let page: Option<u64> = decode(value, plain)?;
    if let Some(n) = page {
        within(n, 1, 200, None, "")?;
    }
    Ok(page)
}

/// `CORE_CONTRACTS` `boolean` (`answerMistake`): a boolean.
fn assert_boolean(value: J) -> Checked<bool> {
    decode(value, plain)
}

/// `setInterfaceZoom`: `auto` or one of the zoom steps (`ZOOM_PERCENTS` in `interfaceZoom.ts`).
fn assert_interface_zoom(value: J) -> Checked<Value> {
    const PERCENTS: [u64; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Zoom {
        Percent(u64),
        Auto(String),
    }
    let invalid = || "Invalid interface zoom.".to_string();
    match decode::<Zoom>(value, plain).map_err(|_| invalid())? {
        Zoom::Auto(text) if text == "auto" => Ok(Value::String(text)),
        Zoom::Percent(percent) if PERCENTS.contains(&percent) => Ok(Value::from(percent)),
        _ => Err(invalid()),
    }
}

/// `CORE_CONTRACTS` `chapterName`: a trimmed name of 1 to 100 characters.
fn assert_chapter_name(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    let text = js::trim(&text);
    min_chars(text, 1, None, "")?;
    max_chars(text, 100, None, "")?;
    Ok(text.to_owned())
}

/// `CORE_CONTRACTS` `pgn` (`exportToLichessStudy`): 1 to 500,000 characters with some content.
fn assert_pgn_text(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    min_chars(&text, 1, None, "")?;
    max_chars(&text, 500_000, None, "")?;
    ensure(!text.trim().is_empty(), None, "", || "some text".into())?;
    Ok(text)
}

/// `CORE_CONTRACTS` `sessionObject` (`saveSession`): any object; the session is checked after it.
fn assert_session_object(value: J) -> Checked<Value> {
    ensure(value.is_object(), None, "", || "an object".into())?;
    Ok(Value::Object(serde_json::Map::new()))
}

/// `CORE_CONTRACTS` `studyId` (`exportToLichessStudy`): nothing (a new study) or a Lichess id.
fn assert_study_id(value: J) -> Checked<String> {
    if matches!(&value, J::Str(text) if text.is_empty()) {
        return Ok(String::new());
    }
    assert_lichess_id(value)
}

/// `apps/desktop/contracts/apiContracts.ts` `completeQuit`: a request id (8-4-4-4-12 hexadecimal
/// digits, in either case).
fn assert_uuid(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    let groups: Vec<&str> = text.split('-').collect();
    const LENGTHS: [usize; 5] = [8, 4, 4, 4, 12];
    let uuid = groups.len() == LENGTHS.len()
        && groups.iter().zip(LENGTHS).all(|(group, length)| {
            group.len() == length && group.bytes().all(|b| b.is_ascii_hexdigit())
        });
    ensure(uuid, None, "", || "a UUID".into())?;
    Ok(text)
}

/// `recordPerformance` name: one of the renderer's timings (`PERFORMANCE_NAMES`).
fn assert_performance_name(value: J) -> Checked<String> {
    let text: String = decode(value, plain)?;
    let known = performance_names().contains(&text);
    ensure(known, None, "", || "a performance timing name".into())?;
    Ok(text)
}

/// `recordPerformance` value: a finite duration from 0 to 300,000 ms.
fn assert_timing_value(value: J) -> Checked<f64> {
    let ms: f64 = decode(value, plain)?;
    finite(ms, None, "")?;
    within(ms, 0.0, 300_000.0, None, "")?;
    Ok(ms)
}

/* ── Requests ── */

/// `reportRendererError`: a fixed page route, a message and a short info string, and no other keys.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(deny_unknown_fields)]
struct RendererError {
    route: String,
    message: String,
    info: String,
}

fn assert_renderer_error(value: J) -> Checked<RendererError> {
    let error: RendererError = decode(value, plain)?;
    one_of(&error.route, RENDERER_ROUTES, None, "route")?;
    max_chars(&error.message, 1000, None, "message")?;
    max_chars(&error.info, 160, None, "info")?;
    Ok(error)
}

/// `assertNewArena`: the arena's clock, schedule and options, and no other keys.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewArena {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    clock_time: f64,
    clock_increment: f64,
    minutes: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_date: Option<f64>,
    variant: String,
    rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

fn assert_new_arena(value: J) -> Checked<NewArena> {
    let mut arena: NewArena = decode(value, |keys| match keys {
        ["clockTime"] => Some("Invalid clock."),
        ["clockIncrement"] => Some("Invalid increment."),
        ["minutes"] => Some("Invalid duration."),
        ["waitMinutes"] => Some("Invalid start."),
        ["variant"] => Some("Invalid variant."),
        _ => None,
    })?;
    if let Some(name) = &mut arena.name {
        *name = js::trim(name).to_owned();
        let too_long = Some("Arena names are limited to 30 characters.");
        max_chars(name, 30, too_long, "name")?;
        let punctuation = Some("Use letters, numbers and simple punctuation.");
        matches(name, patterns::arena_name, punctuation, "name", "")?;
    }
    let clock = Some("Invalid clock.");
    one_of_numbers(arena.clock_time, ARENA_CLOCK_MINUTES, clock, "clockTime")?;
    let increment = Some("Invalid increment.");
    one_of_numbers(
        arena.clock_increment,
        ARENA_INCREMENTS,
        increment,
        "clockIncrement",
    )?;
    let duration = Some("Invalid duration.");
    one_of_numbers(arena.minutes, ARENA_DURATIONS, duration, "minutes")?;
    if let Some(wait) = arena.wait_minutes {
        let start = Some("Invalid start.");
        one_of_numbers(wait, ARENA_WAIT_MINUTES, start, "waitMinutes")?;
    }
    if let Some(start) = arena.start_date {
        let ok = start.is_finite() && start.fract() == 0.0 && start >= 0.0;
        ensure(ok, None, "startDate", || "a time in milliseconds".into())?;
    }
    one_of(
        &arena.variant,
        ARENA_VARIANTS,
        Some("Invalid variant."),
        "variant",
    )?;
    if let Some(password) = &arena.password {
        max_chars(password, 60, None, "password")?;
    }
    if let Some(description) = &arena.description {
        max_chars(description, 2000, None, "description")?;
    }
    Ok(arena)
}

/// `assertExport`: a file name and kind, then that the data is the file it claims to be. A
/// binary export arrives as its length and first bytes; it goes back as `null`, and the caller
/// keeps its own bytes.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct ExportRequest {
    name: String,
    kind: ExportKind,
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ExportKind {
    Gif,
    Png,
    Pgn,
}

fn assert_export(value: J) -> Checked<Value> {
    let data = value.get("data").cloned().unwrap_or(J::Undef);
    let request: ExportRequest = decode(value, |keys| match keys {
        ["kind"] => Some("Invalid export."),
        _ => None,
    })?;
    let file_name = Some("Invalid file name.");
    matches(&request.name, patterns::file_name, file_name, "name", "")?;
    let invalid = || "Invalid export.".to_string();
    let data = match (request.kind, data) {
        (ExportKind::Pgn, J::Str(text)) => {
            max_chars(&text, 2_000_000, None, "data")?;
            Value::String(text)
        }
        (ExportKind::Gif | ExportKind::Png, J::Bytes { len, head }) => {
            let magic: &[u8] = match request.kind {
                ExportKind::Gif => b"GIF8",
                _ => &[137, 80, 78, 71],
            };
            if len > EXPORT_MAX || head.as_slice() != magic {
                return Err(invalid());
            }
            Value::Null
        }
        (_, J::Str(_) | J::Bytes { .. }) => return Err(invalid()),
        _ => return Err("data: invalid type, expected a string or a Uint8Array".into()),
    };
    let mut normalized = out(&request)?;
    if let Value::Object(fields) = &mut normalized {
        fields.insert("data".into(), data);
    }
    Ok(normalized)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct InsightsQuery {
    account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    days: Option<u64>,
}

fn assert_insights_query(value: J) -> Checked<InsightsQuery> {
    let mut query: InsightsQuery = decode(value, |keys| match keys {
        [] => Some("Invalid insights filter."),
        ["account"] => Some(USERNAME),
        ["days"] => Some("Invalid period."),
        _ => None,
    })?;
    query.account = username(&query.account, "account")?;
    if let Some(speed) = &query.speed {
        one_of(speed, EXPLORER_SPEEDS, None, "speed")?;
    }
    if let Some(days) = query.days {
        within(days, 1, 3650, Some("Invalid period."), "days")?;
    }
    Ok(query)
}

/// `assertWatchTarget(value, channels)`: a channel from `channels`, or a game id. A valid
/// channel wins; otherwise a game id is checked; anything else is no game to watch.
fn assert_watch_target(value: J, channels: J) -> Checked<Value> {
    let invalid = || "Invalid game to watch.".to_string();
    if !value.is_object() {
        return Err(invalid());
    }
    let channels: Vec<String> = decode(channels, plain).unwrap_or_default();
    if let Some(channel) = value.get("channel").and_then(J::str)
        && channels.iter().any(|c| c == channel)
    {
        return Ok(serde_json::json!({ "channel": channel }));
    }
    match value.get("gameId").and_then(J::str) {
        Some(id) => {
            game_id(id)?;
            Ok(serde_json::json!({ "gameId": id }))
        }
        None => Err(invalid()),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct OnlineOptions {
    minutes: u64,
    increment: u64,
    color: String,
    rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
}

fn assert_online_options(value: J) -> Checked<OnlineOptions> {
    let mut options: OnlineOptions = decode(value, |keys| match keys {
        [] => Some("Invalid game options."),
        ["minutes"] => Some("Invalid time control."),
        ["increment"] => Some("Invalid increment."),
        ["color"] => Some("Invalid color."),
        ["rated"] => Some("Invalid rated option."),
        ["days"] => Some("Invalid days per move."),
        ["variant"] => Some("Invalid variant."),
        ["fen"] => Some("Invalid position."),
        ["account"] => Some(USERNAME),
        ["target"] => Some("Invalid opponent."),
        _ => None,
    })?;
    within(options.minutes, 1, 180, Some("Invalid time control."), "")?;
    within(options.increment, 0, 180, Some("Invalid increment."), "")?;
    one_of(&options.color, CHALLENGE_COLORS, Some("Invalid color."), "")?;
    if let Some(days) = options.days {
        let msg = Some("Invalid days per move.");
        one_of_numbers(days as f64, CORRESPONDENCE_DAYS, msg, "")?;
    }
    if let Some(variant) = &options.variant {
        one_of(variant, VARIANTS, Some("Invalid variant."), "")?;
    }
    if let Some(position) = &options.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
    }
    if let Some(account) = &options.account {
        options.account = Some(username(account, "account")?);
    }
    if let Some(target) = &options.target {
        let target = js::trim(target);
        let valid = target.is_empty() || patterns::username(target);
        ensure(valid, Some(USERNAME), "", String::new)?;
        options.target = (!target.is_empty()).then(|| target.to_owned());
    }
    Ok(options)
}

/// A theme file: an id, a name and at least one palette.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct Theme {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    light: Option<Palette>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dark: Option<Palette>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct Palette {
    bg: String,
    text: String,
    primary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    elevated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accented: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    border: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    success: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    info: Option<String>,
}

const HEX: &str = "Colors must be hex, like #1a2b3c.";

impl Palette {
    fn check(&self) -> Checked<()> {
        let optional = [
            &self.muted,
            &self.elevated,
            &self.accented,
            &self.border,
            &self.text_muted,
            &self.success,
            &self.warning,
            &self.error,
            &self.info,
        ];
        [&self.bg, &self.text, &self.primary]
            .into_iter()
            .chain(optional.into_iter().flatten())
            .try_for_each(|color| matches(color, patterns::hex_color, Some(HEX), "", ""))
    }
}

fn assert_theme(value: J) -> Checked<Theme> {
    let mut theme: Theme = decode(value, |keys| match keys {
        [] => Some("A theme file must be a JSON object."),
        ["id"] => Some("A theme needs an \"id\"."),
        ["name"] => Some("A theme needs a \"name\"."),
        ["light" | "dark", _] => Some(HEX),
        _ => None,
    })?;
    let id = Some("The \"id\" must be 1–32 characters: a–z, 0–9 and dashes.");
    matches(&theme.id, patterns::theme_id, id, "", "")?;
    theme.name = js::trim(&theme.name).to_owned();
    min_chars(&theme.name, 1, Some("A theme needs a \"name\"."), "")?;
    max_chars(&theme.name, 40, Some("The \"name\" is too long."), "")?;
    for palette in theme.light.iter().chain(&theme.dark) {
        palette.check()?;
    }
    let palette = Some("Add a \"dark\" or \"light\" palette.");
    ensure(
        theme.light.is_some() || theme.dark.is_some(),
        palette,
        "",
        String::new,
    )?;
    Ok(theme)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct Notification {
    kind: String,
    title: String,
    body: String,
}

fn assert_notification(value: J) -> Checked<Notification> {
    const MSG: Msg = Some("Invalid notification.");
    let notification: Notification = decode(value, always!("Invalid notification."))?;
    one_of(&notification.kind, NOTIFICATION_KINDS, MSG, "")?;
    max_chars(&notification.title, 120, MSG, "")?;
    max_chars(&notification.body, 400, MSG, "")?;
    Ok(notification)
}

/// The settings the core keeps (`CORE_SETTINGS_KEYS`), in its order.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct CoreSettings {
    engine_path: String,
    engine_levels: Vec<String>,
    review_auto: String,
    review_on_battery: bool,
    receive_challenges: bool,
    online_chat: bool,
    correspondence_poll: u64,
    cloud_eval: bool,
    voice_history: bool,
}

/// Every setting, in the renderer's order.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct Settings {
    appearance: String,
    board_theme: String,
    light_theme: String,
    dark_theme: String,
    piece_set: String,
    piece_animation: String,
    coordinates: String,
    sound_enabled: bool,
    sound_volume: f64,
    engine_path: String,
    premove: bool,
    promotion: String,
    show_legal_moves: bool,
    notifications_enabled: bool,
    notify_active: bool,
    notify_background: bool,
    notify_opponent_move: bool,
    notify_low_time: bool,
    notify_game_events: bool,
    notify_computer_move: bool,
    notify_sound: bool,
    voice_push_to_talk: bool,
    voice_confirm_moves: bool,
    voice_history: bool,
    update_auto_check: bool,
    update_auto_download: bool,
    update_install_on_quit: bool,
    engine_levels: Vec<String>,
    review_auto: String,
    review_on_battery: bool,
    receive_challenges: bool,
    notify_challenges: bool,
    online_chat: bool,
    correspondence_poll: u64,
    zen_mode: bool,
    blindfold: bool,
    cloud_eval: bool,
    show_opening_name: bool,
    swipe_navigation: bool,
    swipe_indicator: bool,
    puzzle_auto_next: bool,
}

/// The message of each setting.
fn settings_messages(keys: &[&str]) -> Msg {
    Some(match keys {
        ["appearance"] => "Invalid appearance.",
        ["boardTheme"] => "Invalid board theme.",
        ["lightTheme" | "darkTheme"] => "Invalid color theme.",
        ["pieceSet"] => "Invalid piece set.",
        ["pieceAnimation"] => "Invalid piece animation.",
        ["coordinates"] => "Invalid coordinates mode.",
        ["soundEnabled"] => "Invalid sound setting.",
        ["soundVolume"] => "Invalid sound volume.",
        ["enginePath"] => "Invalid engine path.",
        ["premove"] => "Invalid premove setting.",
        ["promotion"] => "Invalid promotion setting.",
        ["showLegalMoves"] => "Invalid legal moves setting.",
        ["voicePushToTalk" | "voiceConfirmMoves" | "voiceHistory"] => "Invalid voice setting.",
        ["updateAutoCheck" | "updateAutoDownload" | "updateInstallOnQuit"] => {
            "Invalid update setting."
        }
        ["engineLevels"] => "Invalid computer levels.",
        ["engineLevels", "[]"] => "Invalid engine level.",
        ["reviewAuto" | "reviewOnBattery"] => "Invalid review setting.",
        ["receiveChallenges"] => "Invalid challenge setting.",
        ["onlineChat"] => "Invalid chat setting.",
        ["correspondencePoll"] => "Invalid correspondence check interval.",
        ["zenMode"] => "Invalid zen mode setting.",
        ["blindfold"] => "Invalid blindfold setting.",
        ["cloudEval"] => "Invalid cloud evaluation setting.",
        ["showOpeningName"] => "Invalid opening name setting.",
        ["swipeNavigation" | "swipeIndicator"] => "Invalid swipe setting.",
        ["puzzleAutoNext"] => "Invalid puzzle setting.",
        ["notificationsEnabled"] => NOTIFY,
        [key] if key.starts_with("notify") => NOTIFY,
        _ => "Invalid settings.",
    })
}

fn theme_setting(id: &str, msg: &'static str) -> Checked<()> {
    matches(id, patterns::theme_id, Some(msg), "", "")
}

/// The core's settings after the engine path, checked in their order. The computer levels are
/// kept in their own order, without repeats; at least one must be chosen.
fn check_core_settings(
    engine_levels: &mut Vec<String>,
    review_auto: &str,
    correspondence_poll: u64,
) -> Checked<()> {
    each(engine_levels, |level| {
        one_of(level, ENGINE_LEVELS, Some("Invalid engine level."), "")
    })?;
    let keep = Some("Keep at least one computer level.");
    min_items(engine_levels, 1, keep, "")?;
    *engine_levels = ENGINE_LEVELS
        .iter()
        .filter(|level| engine_levels.iter().any(|chosen| chosen == *level))
        .map(|level| (*level).to_owned())
        .collect();
    one_of(
        review_auto,
        REVIEW_AUTO,
        Some("Invalid review setting."),
        "",
    )?;
    let poll = Some("Invalid correspondence check interval.");
    within(correspondence_poll, 0, 120, poll, "")
}

fn assert_core_settings(value: J) -> Checked<CoreSettings> {
    let mut s: CoreSettings = decode(value, settings_messages)?;
    max_chars(&s.engine_path, 4096, Some("Invalid engine path."), "")?;
    check_core_settings(&mut s.engine_levels, &s.review_auto, s.correspondence_poll)?;
    Ok(s)
}

fn assert_settings(value: J) -> Checked<Settings> {
    let mut s: Settings = decode(value, settings_messages)?;
    one_of(&s.appearance, APPEARANCES, Some("Invalid appearance."), "")?;
    theme_setting(&s.board_theme, "Invalid board theme.")?;
    theme_setting(&s.light_theme, "Invalid color theme.")?;
    theme_setting(&s.dark_theme, "Invalid color theme.")?;
    theme_setting(&s.piece_set, "Invalid piece set.")?;
    let animation = Some("Invalid piece animation.");
    one_of(&s.piece_animation, PIECE_ANIMATIONS, animation, "")?;
    let coordinates = Some("Invalid coordinates mode.");
    one_of(&s.coordinates, COORDINATE_MODES, coordinates, "")?;
    finite(s.sound_volume, Some("Invalid sound volume."), "")?;
    s.sound_volume = s.sound_volume.clamp(0.0, 1.0);
    max_chars(&s.engine_path, 4096, Some("Invalid engine path."), "")?;
    let promotion = Some("Invalid promotion setting.");
    one_of(&s.promotion, PROMOTION_MODES, promotion, "")?;
    check_core_settings(&mut s.engine_levels, &s.review_auto, s.correspondence_poll)?;
    Ok(s)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct PuzzleRequest {
    account: String,
    angle: String,
    difficulty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

fn assert_puzzle_request(value: J) -> Checked<PuzzleRequest> {
    let mut request: PuzzleRequest = decode(value, |keys| match keys {
        ["account"] => Some("Invalid account."),
        ["angle"] => Some("Invalid puzzle theme."),
        ["difficulty"] => Some("Invalid difficulty."),
        ["color"] => Some("Invalid color."),
        _ => Some("Invalid puzzle request."),
    })?;
    if !request.account.is_empty() {
        request.account = username(&request.account, "")?;
    }
    angle(&request.angle)?;
    let difficulty = Some("Invalid difficulty.");
    one_of(&request.difficulty, PUZZLE_DIFFICULTIES, difficulty, "")?;
    if let Some(color) = &request.color {
        one_of(color, &["white", "black"], Some("Invalid color."), "")?;
    }
    Ok(request)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct PuzzleSolve {
    account: String,
    angle: String,
    id: String,
    win: bool,
    rated: bool,
}

fn assert_puzzle_solve(value: J) -> Checked<PuzzleSolve> {
    let mut solve: PuzzleSolve = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["angle"] => Some("Invalid puzzle theme."),
        ["id"] => Some("Invalid puzzle."),
        _ => Some("Invalid puzzle result."),
    })?;
    solve.account = username(&solve.account, "")?;
    angle(&solve.angle)?;
    matches(
        &solve.id,
        patterns::puzzle_id,
        Some("Invalid puzzle."),
        "",
        "",
    )?;
    Ok(solve)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct LocalQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_rating: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rating: Option<u64>,
    count: u64,
}

fn assert_local_query(value: J) -> Checked<LocalQuery> {
    let query: LocalQuery = decode(value, |keys| match keys {
        ["theme"] => Some("Invalid puzzle theme."),
        ["minRating" | "maxRating"] => Some("Invalid rating."),
        ["count"] => Some("Invalid puzzle count."),
        _ => Some("Invalid puzzle query."),
    })?;
    if let Some(theme) = &query.theme {
        angle(theme)?;
    }
    for rating in query.min_rating.iter().chain(&query.max_rating) {
        within(*rating, 0, 4000, Some("Invalid rating."), "")?;
    }
    within(query.count, 1, 300, Some("Invalid puzzle count."), "")?;
    Ok(query)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct LadderQuery {
    from: u64,
    to: u64,
    count: u64,
}

fn assert_ladder_query(value: J) -> Checked<LadderQuery> {
    let query: LadderQuery = decode(value, |keys| match keys {
        ["from" | "to"] => Some("Invalid rating."),
        ["count"] => Some("Invalid puzzle count."),
        _ => Some("Invalid puzzle query."),
    })?;
    within(query.from, 0, 4000, Some("Invalid rating."), "")?;
    within(query.to, 0, 4000, Some("Invalid rating."), "")?;
    within(query.count, 1, 300, Some("Invalid puzzle count."), "")?;
    Ok(query)
}

/// A saved training run.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct RunInput {
    kind: String,
    variant: String,
    score: f64,
    detail: Detail,
}

/// A run's detail: up to 20 named strings, numbers and flags, in their order.
#[derive(Serialize)]
#[serde(transparent)]
struct Detail(#[serde(with = "entries")] Vec<(String, DetailValue)>);

#[derive(Serialize)]
#[serde(untagged)]
enum DetailValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

impl<'de> Deserialize<'de> for DetailValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Scalar;
        impl Visitor<'_> for Scalar {
            type Value = DetailValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string, number or boolean")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<DetailValue, E> {
                Ok(DetailValue::Flag(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n))
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<DetailValue, E> {
                Ok(DetailValue::Text(text.to_owned()))
            }
        }
        deserializer.deserialize_any(Scalar)
    }
}

impl<'de> Deserialize<'de> for Detail {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        entries::deserialize(deserializer).map(Detail)
    }
}

/// A plain object as an ordered list of entries.
mod entries {
    use super::*;
    use serde::ser::SerializeMap;

    pub fn serialize<S: serde::Serializer, T: Serialize>(
        entries: &[(String, T)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: de::Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<Vec<(String, T)>, D::Error> {
        struct Entries<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Entries<T> {
            type Value = Vec<(String, T)>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(entries)
            }
        }
        // A record is a plain object only; a list is not read by its indices here.
        deserializer.deserialize_any(Entries(std::marker::PhantomData))
    }
}

fn assert_run_input(value: J) -> Checked<RunInput> {
    let run: RunInput = decode(value, |keys| match keys {
        ["kind"] => Some("Invalid kind of run."),
        ["score"] => Some("Invalid score."),
        ["detail", _] => None,
        _ => Some("Invalid run."),
    })?;
    one_of(&run.kind, RUN_KINDS, Some("Invalid kind of run."), "")?;
    matches(
        &run.variant,
        patterns::run_variant,
        Some("Invalid run."),
        "",
        "",
    )?;
    finite(run.score, Some("Invalid score."), "")?;
    within(run.score, 0.0, 1_000_000.0, None, "score")?;
    for (key, value) in &run.detail.0 {
        matches(key, patterns::run_detail_key, Some("Invalid run."), "", "")?;
        let path = format!("detail.{key}");
        match value {
            DetailValue::Text(text) => max_chars(text, 80, None, &path)?,
            DetailValue::Number(n) => finite(*n, None, &path)?,
            DetailValue::Flag(_) => {}
        }
    }
    max_items(&run.detail.0, 20, Some("Invalid run."), "")?;
    Ok(run)
}

/// A puzzle as the trainer shows it (`Puzzle`), as stored in the attempt log.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct AttemptPuzzle {
    id: String,
    fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_move: Option<String>,
    solution: Vec<String>,
    rating: u64,
    themes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plays: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct PuzzleAttempt {
    puzzle: AttemptPuzzle,
    mode: String,
    retry: bool,
    win: bool,
    revealed: bool,
    hinted: bool,
    found: u64,
    wrong: u64,
    total: u64,
    day: String,
}

fn assert_puzzle_attempt(value: J) -> Checked<PuzzleAttempt> {
    const MSG: Msg = Some(ATTEMPT);
    let attempt: PuzzleAttempt = decode(value, |keys| match keys {
        ["puzzle", "lastMove"] | ["puzzle", "solution", "[]"] => Some("Invalid move."),
        ["puzzle", "themes", "[]"] => Some("Invalid puzzle theme."),
        ["puzzle", "gameId"] => Some("Invalid game id."),
        _ => Some(ATTEMPT),
    })?;
    let puzzle = &attempt.puzzle;
    matches(&puzzle.id, patterns::puzzle_id, MSG, "", "")?;
    fen(&puzzle.fen, MSG, MSG, "")?;
    if let Some(last) = &puzzle.last_move {
        uci(last)?;
    }
    each(&puzzle.solution, |m| uci(m))?;
    min_items(&puzzle.solution, 1, MSG, "")?;
    max_items(&puzzle.solution, 64, MSG, "")?;
    within(puzzle.rating, 0, 4000, MSG, "")?;
    each(&puzzle.themes, |theme| angle(theme))?;
    max_items(&puzzle.themes, 40, MSG, "")?;
    if let Some(plays) = puzzle.plays {
        within(plays, 0, MAX_SAFE, MSG, "")?;
    }
    if let Some(id) = &puzzle.game_id {
        game_id(id)?;
    }
    one_of(&attempt.mode, PUZZLE_TRAIN_MODES, MSG, "")?;
    within(attempt.found, 0, 32, MSG, "")?;
    within(attempt.wrong, 0, 32, MSG, "")?;
    within(attempt.total, 1, 32, MSG, "")?;
    local_day(&attempt.day, ATTEMPT)?;
    Ok(attempt)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct PuzzleProgressQuery {
    today: String,
    limit: u64,
}

fn assert_puzzle_progress_query(value: J) -> Checked<PuzzleProgressQuery> {
    const MSG: &str = "Invalid puzzle history query.";
    let query: PuzzleProgressQuery = decode(value, always!(MSG))?;
    local_day(&query.today, MSG)?;
    within(query.limit, 1, 500, Some(MSG), "")?;
    Ok(query)
}

#[derive(Deserialize, Serialize, Default)]
#[serde(expecting = "an object")]
struct BestMoveOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    movetime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chess960: Option<bool>,
}

fn assert_best_move_options(value: J) -> Checked<BestMoveOptions> {
    if matches!(value, J::Undef) {
        return Ok(BestMoveOptions::default());
    }
    let options: BestMoveOptions = decode(value, |keys| match keys {
        ["fen"] => Some("Invalid position."),
        ["movetime"] => Some("Invalid think time."),
        _ => Some("Invalid engine options."),
    })?;
    if let Some(position) = &options.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
    }
    if let Some(movetime) = options.movetime {
        within(movetime, 50, 5000, Some("Invalid think time."), "")?;
    }
    Ok(options)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct AnalysisRequest {
    fen: String,
    lines: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    infinite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    moves: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<u64>,
}

/// `assertAnalysisRequest`: the request's shape, and a position its moves reach.
fn assert_analysis_request(value: J) -> Checked<AnalysisRequest> {
    let request: AnalysisRequest = decode(value, |keys| match keys {
        [] | ["infinite"] => Some("Invalid engine options."),
        ["fen"] => Some("Invalid position."),
        ["lines"] => Some("Invalid number of engine lines."),
        ["rootFen"] => None,
        ["moves", rest @ ..] => move_list_messages(rest, Some("Invalid move list.")),
        ["clientId"] => Some("Invalid analysis request."),
        _ => None,
    })?;
    fen(&request.fen, None, Some("Invalid position."), "fen")?;
    let lines = Some("Invalid number of engine lines.");
    within(request.lines, 1, 5, lines, "")?;
    if let Some(root) = &request.root_fen {
        fen(root, None, None, "rootFen")?;
    }
    if let Some(moves) = &request.moves {
        move_list(moves)?;
    }
    if let Some(client) = request.client_id {
        within(client, 1, MAX_SAFE, Some("Invalid analysis request."), "")?;
    }
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    let moves = request.moves.as_deref().unwrap_or_default();
    let positions = replay::replay_positions(root, moves);
    let normalized = replay::replay_positions(&request.fen, &Vec::<&str>::new())
        .first()
        .map(|p| p.fen.clone());
    let last = positions.last().map(|p| p.fen.as_str());
    if normalized.is_none() || positions.len() != moves.len() + 1 || last != normalized.as_deref() {
        return Err("Invalid analysis position or move history.".into());
    }
    Ok(request)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    fen: String,
    moves: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
}

/// `assertReviewRequest`: the request's shape, and moves that replay from its position.
fn assert_review_request(value: J) -> Checked<ReviewRequest> {
    let mut request: ReviewRequest = decode(value, |keys| match keys {
        ["fen"] => Some("Invalid position."),
        ["moves", rest @ ..] => move_list_messages(rest, Some("Invalid move list.")),
        ["gameId"] => Some("Invalid game id."),
        ["account"] => Some(USERNAME),
        _ => Some("Invalid review request."),
    })?;
    fen(&request.fen, None, Some("Invalid position."), "fen")?;
    move_list(&request.moves)?;
    if let Some(id) = &request.game_id {
        game_id(id)?;
    }
    if let Some(account) = &request.account {
        request.account = Some(username(account, "account")?);
    }
    if replay::replay_positions_count(&request.fen, &request.moves) != request.moves.len() + 1 {
        return Err("Invalid game position or move history.".into());
    }
    Ok(request)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct VoiceAttempt {
    source: String,
    heard: String,
    confidence: f64,
    words: Vec<VoiceWord>,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parsed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_of: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct VoiceWord {
    word: String,
    conf: f64,
}

/// A recognizer's confidence, from 0 to 1.
fn score(n: f64, path: &str) -> Checked<()> {
    within(n, 0.0, 1.0, None, path)
}

fn assert_voice_attempt(value: J) -> Checked<VoiceAttempt> {
    let attempt: VoiceAttempt = decode(value, |keys| match keys {
        ["words"] | ["words", "[]"] => None,
        _ => Some(VOICE),
    })?;
    one_of(&attempt.source, VOICE_SOURCES, Some(VOICE), "")?;
    max_chars(&attempt.heard, 200, None, "heard")?;
    score(attempt.confidence, "confidence")?;
    for word in &attempt.words {
        max_chars(&word.word, 40, None, "words.word")?;
        score(word.conf, "words.conf")?;
    }
    max_items(&attempt.words, 40, Some(VOICE), "")?;
    one_of(&attempt.outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
    if let Some(parsed) = &attempt.parsed {
        max_chars(parsed, 120, None, "parsed")?;
    }
    if let Some(expected) = &attempt.expected {
        max_chars(expected, 20, None, "expected")?;
    }
    if let Some(position) = &attempt.fen {
        fen(position, None, Some("Invalid position."), "fen")?;
    }
    if let Some(retry) = attempt.retry_of {
        within(retry, 1, MAX_SAFE, Some(VOICE), "")?;
    }
    Ok(attempt)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct VoiceUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
}

fn assert_voice_update(value: J) -> Checked<VoiceUpdate> {
    let update: VoiceUpdate = decode(value, always!(VOICE))?;
    if let Some(outcome) = &update.outcome {
        one_of(outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
    }
    if let Some(expected) = &update.expected {
        max_chars(expected, 20, None, "expected")?;
    }
    Ok(update)
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct GamePageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<GameResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rated: Option<bool>,
    offset: u64,
    limit: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum GameResult {
    Win,
    Loss,
    Draw,
}

fn assert_game_page_query(value: J) -> Checked<GamePageQuery> {
    let mut query: GamePageQuery = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["offset"] => Some("Invalid game offset."),
        ["limit"] => Some("Invalid game page size."),
        _ => None,
    })?;
    if let Some(account) = &query.account {
        query.account = Some(username(account, "account")?);
    }
    within(query.offset, 0, MAX_SAFE, Some("Invalid game offset."), "")?;
    within(query.limit, 1, 100, Some("Invalid game page size."), "")?;
    Ok(query)
}

/// `assertStudySyncRequest`: a study's account, id and both PGN copies, bounded.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct StudySyncRequest {
    account: Option<String>,
    study_id: Option<String>,
    baseline: Option<String>,
    pgn: Option<String>,
}

const STUDY_TEXT: &str = "The study is empty or too large to upload.";

fn assert_study_sync_request(value: J) -> Checked<StudySyncRequest> {
    if !value.is_object() {
        return Err("Invalid study synchronization.".into());
    }
    let mut request: StudySyncRequest = decode(value, |keys| match keys {
        ["account"] => Some(USERNAME),
        ["studyId"] => Some("Invalid id."),
        ["baseline" | "pgn"] => Some(STUDY_TEXT),
        _ => Some("Invalid study synchronization."),
    })?;
    let account = request.account.as_deref().ok_or(USERNAME)?;
    request.account = Some(username(account, "")?);
    lichess_id(
        request.study_id.as_deref().ok_or("Invalid id.")?,
        "Invalid id.",
    )?;
    let usable = |text: &Option<String>| {
        text.as_deref()
            .is_some_and(|t| !js::trim(t).is_empty() && js::utf16_len(t) <= 500_000)
    };
    ensure(
        usable(&request.baseline) && usable(&request.pgn),
        Some(STUDY_TEXT),
        "",
        String::new,
    )?;
    Ok(request)
}

#[derive(Deserialize, Serialize, Default)]
#[serde(expecting = "an object")]
struct LookupOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    speeds: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratings: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    player: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modes: Option<Vec<GameMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Side {
    White,
    Black,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum GameMode {
    Rated,
    Casual,
}

fn assert_lookup_options(value: J) -> Checked<LookupOptions> {
    if matches!(value, J::Undef) {
        return Ok(LookupOptions::default());
    }
    let options: LookupOptions = decode(value, |keys| match keys {
        [] => Some("Invalid explorer filters."),
        _ => None,
    })?;
    if let Some(speeds) = &options.speeds {
        each(speeds, |speed| {
            one_of(speed, EXPLORER_SPEEDS, None, "speeds")
        })?;
        max_items(speeds, 6, None, "speeds")?;
    }
    if let Some(ratings) = &options.ratings {
        each(ratings, |rating| {
            one_of_numbers(*rating as f64, EXPLORER_RATINGS, None, "ratings")
        })?;
        max_items(ratings, 9, None, "ratings")?;
    }
    if let Some(player) = &options.player {
        matches(
            player,
            patterns::username,
            None,
            "player",
            "a Lichess username",
        )?;
    }
    if let Some(modes) = &options.modes {
        max_items(modes, 2, None, "modes")?;
    }
    if let Some(since) = &options.since {
        matches(
            since,
            patterns::since,
            None,
            "since",
            "a year or a year and month",
        )?;
    }
    Ok(options)
}

/* ── Library documents ── */

/// A study's name.
fn study_name(name: &str, path: &str) -> Checked<()> {
    max_chars(name, 200, None, path)
}

/// A local study or chapter id: 1 to 80 characters.
fn local_id(id: &str, path: &str) -> Checked<()> {
    min_chars(id, 1, None, path)?;
    max_chars(id, 80, None, path)
}

fn study_pgn(pgn: &str, path: &str) -> Checked<()> {
    max_chars(pgn, MAX_DOCUMENT, None, path)
}

fn study_account(account: &str) -> Checked<()> {
    max_chars(account, 40, None, "account")
}

fn study_lichess_id(id: &str, path: &str) -> Checked<()> {
    matches(id, patterns::lichess_id, None, path, "a Lichess id")
}

/// The study commands, by their `op`.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase", expecting = "a study command")]
enum StudyOp {
    Save,
    SaveChapters,
    AddChapter,
    RenameChapter,
    DuplicateChapter,
    RemoveChapter,
    MarkCloud,
    Offline,
    Remove,
    Restore,
    Rename,
    Duplicate,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct ChapterInput {
    name: String,
    pgn: String,
}

/// 1 to `MAX_CHAPTERS` chapters, each with a name and a PGN.
fn chapters(list: &[ChapterInput]) -> Checked<()> {
    for chapter in list {
        study_name(&chapter.name, "chapters.name")?;
        study_pgn(&chapter.pgn, "chapters.pgn")?;
    }
    min_items(list, 1, None, "chapters")?;
    max_items(list, MAX_CHAPTERS, None, "chapters")
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SaveStudy {
    op: String,
    name: String,
    pgn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chapter_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct SaveChapters {
    op: String,
    name: String,
    chapters: Vec<ChapterInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// `addChapter`, `rename`: a study and a name.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct NamedStudy {
    op: String,
    id: String,
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct RenameChapter {
    op: String,
    id: String,
    chapter_id: String,
    name: String,
}

/// `duplicateChapter`, `removeChapter`: a study's chapter.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct StudyChapter {
    op: String,
    id: String,
    chapter_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct MarkCloud {
    op: String,
    id: String,
    account: String,
    remote_id: String,
    baseline: String,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct Offline {
    op: String,
    account: String,
    remote: RemoteStudy,
    chapters: Vec<ChapterInput>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct RemoteStudy {
    id: String,
    name: String,
}

/// `remove`, `duplicate`: a study.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
struct StudyRef {
    op: String,
    id: String,
}

/// `studyCommandShape`: one study command, chosen by its `op`, with the fields that command
/// takes (others are dropped). A study to restore is passed on as it is; the core checks it.
fn assert_study_command_shape(value: J) -> Checked<Value> {
    if !value.is_object() {
        let error = decode::<SaveStudy>(value, plain).err();
        return Err(error.unwrap_or_else(|| "invalid type, expected an object".into()));
    }
    let tag = value.get("op").cloned().unwrap_or(J::Undef);
    let op: StudyOp = decode(tag, plain).map_err(|error| format!("op: {error}"))?;
    match op {
        StudyOp::Save => {
            let save: SaveStudy = decode(value, plain)?;
            study_name(&save.name, "name")?;
            study_pgn(&save.pgn, "pgn")?;
            for id in save.id.iter().chain(&save.chapter_id) {
                local_id(id, "id")?;
            }
            out(&save)
        }
        StudyOp::SaveChapters => {
            let save: SaveChapters = decode(value, plain)?;
            study_name(&save.name, "name")?;
            chapters(&save.chapters)?;
            if let Some(id) = &save.id {
                local_id(id, "id")?;
            }
            out(&save)
        }
        StudyOp::AddChapter | StudyOp::Rename => {
            let named: NamedStudy = decode(value, plain)?;
            local_id(&named.id, "id")?;
            study_name(&named.name, "name")?;
            out(&named)
        }
        StudyOp::RenameChapter => {
            let rename: RenameChapter = decode(value, plain)?;
            local_id(&rename.id, "id")?;
            local_id(&rename.chapter_id, "chapterId")?;
            study_name(&rename.name, "name")?;
            out(&rename)
        }
        StudyOp::DuplicateChapter | StudyOp::RemoveChapter => {
            let chapter: StudyChapter = decode(value, plain)?;
            local_id(&chapter.id, "id")?;
            local_id(&chapter.chapter_id, "chapterId")?;
            out(&chapter)
        }
        StudyOp::MarkCloud => {
            let cloud: MarkCloud = decode(value, plain)?;
            local_id(&cloud.id, "id")?;
            study_account(&cloud.account)?;
            study_lichess_id(&cloud.remote_id, "remoteId")?;
            study_pgn(&cloud.baseline, "baseline")?;
            out(&cloud)
        }
        StudyOp::Offline => {
            let offline: Offline = decode(value, plain)?;
            study_account(&offline.account)?;
            study_lichess_id(&offline.remote.id, "remote.id")?;
            study_name(&offline.remote.name, "remote.name")?;
            chapters(&offline.chapters)?;
            out(&offline)
        }
        StudyOp::Remove | StudyOp::Duplicate => {
            let study: StudyRef = decode(value, plain)?;
            local_id(&study.id, "id")?;
            out(&study)
        }
        StudyOp::Restore => {
            let study = value.get("study").cloned().ok_or("missing field `study`")?;
            let mut command = serde_json::Map::new();
            command.insert("op".into(), Value::from("restore"));
            if study != J::Undef {
                command.insert("study".into(), study.into_value());
            }
            Ok(Value::Object(command))
        }
    }
}

/// A library entry's id: 1 to 80 characters.
fn assert_library_id(value: J) -> Checked<String> {
    let id: String = decode(value, plain)?;
    local_id(&id, "")?;
    Ok(id)
}

/// A repertoire's key (`studyId:color`): 1 to 200 characters.
fn assert_repertoire_key(value: J) -> Checked<String> {
    let key: String = decode(value, plain)?;
    min_chars(&key, 1, None, "")?;
    max_chars(&key, 200, None, "")?;
    Ok(key)
}

fn assert_session_kind(value: J) -> Checked<String> {
    picked(value, SESSION_KINDS, None)
}

fn assert_side(value: J) -> Checked<String> {
    picked(value, &["white", "black"], None)
}

/// An earlier release's renderer storage: each known key's document as text, and no other key.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, expecting = "an object")]
struct LegacyDocuments {
    #[serde(rename = "kchess:studies:v1", skip_serializing_if = "Option::is_none")]
    studies: Option<String>,
    #[serde(
        rename = "kchess:game-history:v1",
        skip_serializing_if = "Option::is_none"
    )]
    game_history: Option<String>,
    #[serde(rename = "kchess:mistakes:v1", skip_serializing_if = "Option::is_none")]
    mistakes: Option<String>,
    #[serde(rename = "kchess:analysis:v1", skip_serializing_if = "Option::is_none")]
    analysis: Option<String>,
    #[serde(rename = "kchess:local:v1", skip_serializing_if = "Option::is_none")]
    local: Option<String>,
    #[serde(rename = "kchess:computer:v1", skip_serializing_if = "Option::is_none")]
    computer: Option<String>,
    #[serde(
        rename = "kchess:history-session:computer",
        skip_serializing_if = "Option::is_none"
    )]
    history_computer: Option<String>,
    #[serde(
        rename = "kchess:history-session:board",
        skip_serializing_if = "Option::is_none"
    )]
    history_board: Option<String>,
    #[serde(
        rename = "kchess:history-session:clock",
        skip_serializing_if = "Option::is_none"
    )]
    history_clock: Option<String>,
    #[serde(
        rename = "kchess:tournaments-joined",
        skip_serializing_if = "Option::is_none"
    )]
    tournaments_joined: Option<String>,
    #[serde(
        rename = "kchess:repertoire-misses",
        skip_serializing_if = "Option::is_none"
    )]
    repertoire_misses: Option<String>,
}

fn assert_legacy_documents(value: J) -> Checked<LegacyDocuments> {
    let documents: LegacyDocuments = decode(value, plain)?;
    let all = [
        &documents.studies,
        &documents.game_history,
        &documents.mistakes,
        &documents.analysis,
        &documents.local,
        &documents.computer,
        &documents.history_computer,
        &documents.history_board,
        &documents.history_clock,
        &documents.tournaments_joined,
        &documents.repertoire_misses,
    ];
    for (key, text) in LEGACY_DOCUMENT_KEYS.iter().zip(all) {
        if let Some(text) = text {
            max_chars(text, MAX_DOCUMENT, None, key)?;
        }
    }
    Ok(documents)
}

/* ── The table ── */

macro_rules! validators {
    ($($name:literal => $check:path,)*) => {
        /// The validators by their TypeScript names.
        pub fn validator(method: &str) -> Option<Validator> {
            Some(match method {
                $($name => {
                    let run: Validator = |args| out(&$check(first(args))?);
                    run
                })*
                "assertWatchTarget" => {
                    let run: Validator = |args| {
                        let mut args = args.into_iter();
                        let value = args.next().unwrap_or(J::Undef);
                        assert_watch_target(value, args.next().unwrap_or(J::Undef))
                    };
                    run
                }
                _ => return None,
            })
        }
    };
}

validators! {
    "assertUuid" => assert_uuid,
    "assertPerformanceName" => assert_performance_name,
    "assertTimingValue" => assert_timing_value,
    "assertVoiceGrammar" => assert_voice_grammar,
    "assertSampleRate" => assert_sample_rate,
    "assertVoiceAudio" => assert_voice_audio,
    "assertRendererError" => assert_renderer_error,
    "assertUsername" => assert_username,
    "assertGameId" => assert_game_id,
    "assertGameIds" => assert_game_ids,
    "assertUci" => assert_uci,
    "assertMoves" => assert_moves,
    "assertLevel" => assert_level,
    "assertUsernames" => assert_usernames,
    "assertFriendList" => assert_friend_list,
    "assertAction" => assert_action,
    "assertChatRoom" => assert_chat_room,
    "assertChatText" => assert_chat_text,
    "assertNewArena" => assert_new_arena,
    "assertMessageText" => assert_message_text,
    "assertDeclineReason" => assert_decline_reason,
    "assertTournamentSystem" => assert_tournament_system,
    "assertTournamentId" => assert_tournament_id,
    "assertTournamentPassword" => assert_tournament_password,
    "assertExport" => assert_export,
    "assertInsightsQuery" => assert_insights_query,
    "assertPerfType" => assert_perf_type,
    "assertLichessId" => assert_lichess_id,
    "assertOptionalAccount" => assert_optional_account,
    "assertOnlineOptions" => assert_online_options,
    "assertTheme" => assert_theme,
    "assertNotification" => assert_notification,
    "assertCoreSettings" => assert_core_settings,
    "assertSettings" => assert_settings,
    "assertPuzzleRequest" => assert_puzzle_request,
    "assertPuzzleSolve" => assert_puzzle_solve,
    "assertDays" => assert_days,
    "assertActivityMax" => assert_activity_max,
    "assertLocalQuery" => assert_local_query,
    "assertLadderQuery" => assert_ladder_query,
    "assertRunKind" => assert_run_kind,
    "assertRunInput" => assert_run_input,
    "assertPuzzleAttempt" => assert_puzzle_attempt,
    "assertPuzzleProgressQuery" => assert_puzzle_progress_query,
    "assertBestMoveOptions" => assert_best_move_options,
    "assertAnalysisRequest" => assert_analysis_request,
    "assertReviewRequest" => assert_review_request,
    "assertReviewKey" => assert_review_key,
    "assertVoiceAttempt" => assert_voice_attempt,
    "assertVoiceUpdate" => assert_voice_update,
    "assertVoiceId" => assert_voice_id,
    "assertVoiceLimit" => assert_voice_limit,
    "assertGamePageQuery" => assert_game_page_query,
    "assertStudySyncRequest" => assert_study_sync_request,
    "assertBroadcastQuery" => assert_broadcast_query,
    "assertLookupOptions" => assert_lookup_options,
    "assertLines" => assert_lines,
    "assertLookupKind" => assert_lookup_kind,
    "assertOptionalPage" => assert_optional_page,
    "assertBoolean" => assert_boolean,
    "assertInterfaceZoom" => assert_interface_zoom,
    "assertChapterName" => assert_chapter_name,
    "assertPgnText" => assert_pgn_text,
    "assertSessionObject" => assert_session_object,
    "assertStudyId" => assert_study_id,
    "assertStudyCommandShape" => assert_study_command_shape,
    "assertLibraryId" => assert_library_id,
    "assertRepertoireKey" => assert_repertoire_key,
    "assertSessionKind" => assert_session_kind,
    "assertSide" => assert_side,
    "assertLegacyDocuments" => assert_legacy_documents,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    fn run(method: &str, value: J) -> Checked<Value> {
        validator(method).expect(method)(vec![value])
    }

    fn settings() -> Vec<(&'static str, J)> {
        let mut fields = vec![
            ("appearance", J::Str("system".into())),
            ("boardTheme", J::Str("classic".into())),
            ("lightTheme", J::Str("light".into())),
            ("darkTheme", J::Str("dark".into())),
            ("pieceSet", J::Str("cburnett".into())),
            ("pieceAnimation", J::Str("normal".into())),
            ("coordinates", J::Str("inside".into())),
            ("soundVolume", J::Num(7.0)),
            ("enginePath", J::Str(String::new())),
            ("promotion", J::Str("ask".into())),
            (
                "engineLevels",
                J::Arr(vec![J::Str("gm".into()), J::Str("club".into())]),
            ),
            ("reviewAuto", J::Str("off".into())),
            ("correspondencePoll", J::Num(30.0)),
        ];
        for flag in [
            "soundEnabled",
            "premove",
            "showLegalMoves",
            "notificationsEnabled",
            "notifyActive",
            "notifyBackground",
            "notifyOpponentMove",
            "notifyLowTime",
            "notifyGameEvents",
            "notifyComputerMove",
            "notifySound",
            "voicePushToTalk",
            "voiceConfirmMoves",
            "voiceHistory",
            "updateAutoCheck",
            "updateAutoDownload",
            "updateInstallOnQuit",
            "reviewOnBattery",
            "receiveChallenges",
            "notifyChallenges",
            "onlineChat",
            "zenMode",
            "blindfold",
            "cloudEval",
            "showOpeningName",
            "swipeNavigation",
            "swipeIndicator",
            "puzzleAutoNext",
        ] {
            fields.push((flag, J::Bool(true)));
        }
        fields
    }

    #[test]
    fn settings_normalize_in_the_renderers_order() {
        let normalized = run("assertSettings", obj(settings())).unwrap();
        assert_eq!(normalized["soundVolume"], json!(1));
        assert_eq!(normalized["engineLevels"], json!(["club", "gm"]));
        let keys: Vec<&String> = normalized.as_object().unwrap().keys().collect();
        assert_eq!(keys[..3], ["appearance", "boardTheme", "lightTheme"]);
    }

    #[test]
    fn each_setting_keeps_its_message() {
        for (key, value, message) in [
            ("notificationsEnabled", J::Num(1.0), NOTIFY),
            ("notifyLowTime", J::Null, NOTIFY),
            (
                "engineLevels",
                J::Arr(vec![J::Num(1.0)]),
                "Invalid engine level.",
            ),
            (
                "engineLevels",
                J::Arr(vec![]),
                "Keep at least one computer level.",
            ),
            ("soundVolume", J::Num(f64::NAN), "Invalid sound volume."),
            (
                "soundVolume",
                J::Num(f64::INFINITY),
                "Invalid sound volume.",
            ),
            (
                "correspondencePoll",
                J::Num(1.5),
                "Invalid correspondence check interval.",
            ),
            ("lightTheme", J::Str("Bad".into()), "Invalid color theme."),
        ] {
            let mut fields = settings();
            fields.iter_mut().find(|(k, _)| *k == key).unwrap().1 = value;
            assert_eq!(
                run("assertSettings", obj(fields)),
                Err(message.into()),
                "{key}"
            );
        }
        let mut missing = settings();
        missing.retain(|(k, _)| *k != "zenMode");
        assert_eq!(
            run("assertSettings", obj(missing)),
            Err("Invalid settings.".into())
        );
    }

    #[test]
    fn nested_values_take_their_own_message() {
        let palette = obj(vec![
            ("bg", J::Num(1.0)),
            ("text", J::Str("#fff".into())),
            ("primary", J::Str("#000".into())),
        ]);
        let theme = obj(vec![
            ("id", J::Str("night".into())),
            ("name", J::Str("Night".into())),
            ("dark", palette),
        ]);
        assert_eq!(run("assertTheme", theme), Err(HEX.into()));
        let moves = obj(vec![
            ("fen", J::Str("x".into())),
            ("moves", J::Arr(vec![J::Num(1.0)])),
        ]);
        assert_eq!(
            run("assertReviewRequest", moves),
            Err("Invalid move.".into())
        );
    }

    #[test]
    fn lists_are_objects_of_their_indices_and_binary_data_is_not_text() {
        assert_eq!(run("assertBestMoveOptions", J::Arr(vec![])), Ok(json!({})));
        assert_eq!(run("assertBestMoveOptions", J::Undef), Ok(json!({})));
        let bytes = J::Bytes {
            len: 2,
            head: b"hi".to_vec(),
        };
        assert_eq!(run("assertChatText", bytes), Err("Invalid message.".into()));
    }

    #[test]
    fn default_messages_name_the_rejected_value() {
        assert_eq!(
            run("assertNewArena", obj(vec![])),
            Err("missing field `clockTime`".into())
        );
        let speeds = obj(vec![("speeds", J::Arr(vec![J::Str("bogus".into())]))]);
        assert!(
            run("assertLookupOptions", speeds)
                .unwrap_err()
                .starts_with("speeds: invalid value, expected one of ultraBullet")
        );
        let extra = obj(vec![
            ("route", J::Str("/".into())),
            ("message", J::Str("m".into())),
            ("info", J::Str("i".into())),
            ("extra", J::Num(1.0)),
        ]);
        assert!(
            run("assertRendererError", extra)
                .unwrap_err()
                .starts_with("unknown field `extra`")
        );
    }

    #[test]
    fn run_details_keep_their_order_and_integers() {
        let run_input = obj(vec![
            ("kind", J::Str("storm".into())),
            ("variant", J::Str(String::new())),
            ("score", J::Num(12.0)),
            (
                "detail",
                obj(vec![("z", J::Num(1.0)), ("a", J::Str("x".into()))]),
            ),
        ]);
        let normalized = run("assertRunInput", run_input).unwrap();
        assert_eq!(normalized["score"], json!(12));
        assert_eq!(
            serde_json::to_string(&normalized["detail"]).unwrap(),
            r#"{"z":1,"a":"x"}"#
        );
    }
}
