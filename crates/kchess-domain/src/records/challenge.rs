//! The reader of a challenge event from an account's event stream (`readChallengeEvent`,
//! the challenges service of the TypeScript core), a pure rule beside `validateOnlineEvent`.
//!
//! The reader keeps the TypeScript schema's rules (valibot): unknown keys pass, an optional key
//! may be absent but not null, lengths count UTF-16 units, and a malformed event is not a
//! challenge at all. The variant mapping comes from `crate::online_game`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use super::{Out, arg};

/// `GAME_ID` of `domain/patterns.ts`: `^[a-zA-Z0-9]{8,12}$`.
fn is_game_id(text: &str) -> bool {
    (8..=12).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// A player as a challenge names them (`PlayerRef`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisional: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

/// `ChallengeTimeControl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TimeControl {
    Clock { limit: Number, increment: Number },
    Correspondence { days: Number },
    Unlimited,
}

/// `ChallengeInfo`: a pending challenge to or from one of the connected accounts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeInfo {
    pub id: String,
    /// The connected account it was sent to (incoming) or from (outgoing).
    pub account: String,
    /// `in` or `out`.
    pub direction: String,
    pub opponent: PlayerRef,
    pub variant: String,
    pub variant_name: String,
    pub rated: bool,
    pub speed: String,
    pub time_control: TimeControl,
    /// `random`, `white` or `black`.
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rematch_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_fen: Option<String>,
    pub playable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Milliseconds since the Unix epoch, as `Date.now()` gives.
    pub received_at: i64,
}

/// A challenge event: its type (`challenge`, `challengeCanceled` or `challengeDeclined`) and the
/// challenge it names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeEvent {
    pub kind: String,
    pub info: ChallengeInfo,
}

type Object = Map<String, Value>;

/// Characters a JavaScript string counts for `maxLength` (UTF-16 code units).
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `v.optional(check)` for one key: `Some(None)` when absent, `Some(Some(_))` when it passes,
/// `None` when it is present and fails (null fails too).
fn optional<'a, T>(
    object: &'a Object,
    key: &str,
    check: impl Fn(&'a Value) -> Option<T>,
) -> Option<Option<T>> {
    match object.get(key) {
        None => Some(None),
        Some(value) => check(value).map(Some),
    }
}

/// `v.string()` within `max` UTF-16 units.
fn text(value: &Value, max: usize) -> Option<&str> {
    value.as_str().filter(|text| units(text) <= max)
}

/// `v.pipe(v.number(), v.finite(), v.minValue(0))`.
fn non_negative(value: &Value) -> Option<Number> {
    let number = value.as_number()?;
    number.as_f64().filter(|n| *n >= 0.0)?;
    Some(number.clone())
}

/// `v.picklist(options)`.
fn picklist<'a>(value: &'a Value, options: &[&str]) -> Option<&'a str> {
    value.as_str().filter(|text| options.contains(text))
}

fn game_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| is_game_id(text))
}

/// `userSchema`.
struct User {
    id: String,
    name: String,
    rating: Option<Number>,
    title: Option<String>,
    provisional: Option<bool>,
    online: Option<bool>,
}

fn user(value: &Value) -> Option<User> {
    let object = value.as_object()?;
    let id = text(object.get("id")?, 40)?.to_string();
    let name = text(object.get("name")?, 40)?.to_string();
    let rating = optional(object, "rating", non_negative_any)?;
    // `v.optional(v.nullable(text(8)))`: a null title is read as no title.
    let title = optional(object, "title", |v| {
        if v.is_null() {
            Some(None)
        } else {
            text(v, 8).map(|t| Some(t.to_string()))
        }
    })?
    .flatten();
    let provisional = optional(object, "provisional", Value::as_bool)?;
    let online = optional(object, "online", Value::as_bool)?;
    Some(User {
        id,
        name,
        rating,
        title,
        provisional,
        online,
    })
}

/// A finite number of any sign (`v.pipe(v.number(), v.finite())`).
fn non_negative_any(value: &Value) -> Option<Number> {
    value.as_number().cloned()
}

/// `nullable(user)` under a required key.
fn nullable_user(object: &Object, key: &str) -> Option<Option<User>> {
    let value = object.get(key)?;
    if value.is_null() {
        Some(None)
    } else {
        user(value).map(Some)
    }
}

/// The parts of the challenge schema the reader keeps; the rest is validated and dropped.
struct Challenge {
    id: String,
    challenger: Option<User>,
    dest_user: Option<User>,
    variant_key: String,
    variant_name: Option<String>,
    rated: bool,
    speed: String,
    time_control: TimeControl,
    color: Option<String>,
    direction: Option<String>,
    initial_fen: Option<String>,
    rematch_of: Option<String>,
}

const COLORS: &[&str] = &["white", "black", "random"];
const DIRECTIONS: &[&str] = &["in", "out"];

fn challenge(object: &Object) -> Option<Challenge> {
    let id = game_id(object.get("id")?)?.to_string();
    // `v.optional(text(20))`: a status is checked for length and otherwise unused.
    optional(object, "status", |v| text(v, 20))?;
    let challenger = nullable_user(object, "challenger")?;
    let dest_user = nullable_user(object, "destUser")?;
    let variant = object.get("variant")?.as_object()?;
    let variant_key = text(variant.get("key")?, 30)?.to_string();
    let variant_name = optional(variant, "name", |v| text(v, 40))?.map(str::to_string);
    let rated = object.get("rated")?.as_bool()?;
    let speed = text(object.get("speed")?, 20)?.to_string();
    let time_control = time_control(object.get("timeControl")?)?;
    let color = optional(object, "color", |v| picklist(v, COLORS))?.map(str::to_string);
    let direction = optional(object, "direction", |v| picklist(v, DIRECTIONS))?.map(str::to_string);
    let initial_fen = optional(object, "initialFen", |v| text(v, 100))?.map(str::to_string);
    let rematch_of = optional(object, "rematchOf", game_id)?.map(str::to_string);
    Some(Challenge {
        id,
        challenger,
        dest_user,
        variant_key,
        variant_name,
        rated,
        speed,
        time_control,
        color,
        direction,
        initial_fen,
        rematch_of,
    })
}

/// `v.variant('type', [clock, correspondence, unlimited])`.
fn time_control(value: &Value) -> Option<TimeControl> {
    let object = value.as_object()?;
    match object.get("type")?.as_str()? {
        "clock" => {
            let limit = optional(object, "limit", non_negative)?;
            let increment = optional(object, "increment", non_negative)?;
            Some(TimeControl::Clock {
                limit: limit.unwrap_or_else(|| Number::from(0)),
                increment: increment.unwrap_or_else(|| Number::from(0)),
            })
        }
        "correspondence" => {
            let days = optional(object, "daysPerTurn", non_negative)?;
            Some(TimeControl::Correspondence {
                days: days.unwrap_or_else(|| Number::from(0)),
            })
        }
        "unlimited" => Some(TimeControl::Unlimited),
        _ => None,
    }
}

/// `readChallengeEvent(account, raw, now)`: the challenge a stream event carries, seen from
/// `account`'s side; None when the event is not a challenge event or does not match the schema.
pub fn read_challenge_event(account: &str, raw: &Value, now: i64) -> Option<ChallengeEvent> {
    let object = raw.as_object()?;
    let kind = picklist(
        object.get("type")?,
        &["challenge", "challengeCanceled", "challengeDeclined"],
    )?
    .to_string();
    let parsed = challenge(object.get("challenge")?.as_object()?)?;
    // `compat` is optional; a present one must be an object with an optional boolean `board`.
    let board_ok = match object.get("compat") {
        None => true,
        Some(compat) => {
            let compat = compat.as_object()?;
            let board = optional(compat, "board", Value::as_bool)?;
            board != Some(false)
        }
    };

    let ours = account.to_lowercase();
    let outgoing = parsed.direction.as_deref() == Some("out")
        || (parsed.direction.is_none()
            && parsed
                .challenger
                .as_ref()
                .is_some_and(|challenger| challenger.id.to_lowercase() == ours));
    let other = if outgoing {
        parsed.dest_user.as_ref()
    } else {
        parsed.challenger.as_ref()
    };
    let variant_key = parsed.variant_key.clone();
    let variant_name = parsed
        .variant_name
        .clone()
        .unwrap_or_else(|| variant_key.clone());
    let playable_variant = crate::online_game::variant_from_lichess(Some(variant_key.as_str()));
    let problem = if playable_variant.is_none() {
        Some(format!(
            "{variant_name} needs a piece-drop board KChess does not have yet."
        ))
    } else if !board_ok {
        Some(format!(
            "Lichess does not let third-party apps play {} games.",
            parsed.speed
        ))
    } else {
        None
    };
    let opponent = PlayerRef {
        name: other
            .map(|user| user.name.clone())
            .unwrap_or_else(|| "Anyone".to_string()),
        rating: other.and_then(|user| user.rating.clone()),
        title: other.and_then(|user| user.title.clone()),
        provisional: other.and_then(|user| user.provisional),
        online: other.and_then(|user| user.online),
    };
    let initial_fen = parsed.initial_fen.clone().filter(|fen| fen != "startpos");
    let info = ChallengeInfo {
        id: parsed.id,
        account: account.to_string(),
        direction: if outgoing { "out" } else { "in" }.to_string(),
        opponent,
        variant: variant_key,
        variant_name,
        rated: parsed.rated,
        speed: parsed.speed,
        time_control: parsed.time_control,
        color: parsed.color.unwrap_or_else(|| "random".to_string()),
        rematch_of: parsed.rematch_of,
        initial_fen,
        playable: problem.is_none(),
        problem,
        received_at: now,
    };
    Some(ChallengeEvent { kind, info })
}

/// This module's methods for `records::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "readChallengeEvent" => {
            let account = arg(args, 0).and_then(Value::as_str).unwrap_or_default();
            let now = arg(args, 2).and_then(Value::as_i64).unwrap_or(0);
            match arg(args, 1) {
                None => Ok(Value::Null),
                Some(raw) => Ok(match read_challenge_event(account, raw, now) {
                    Some(event) => serde_json::to_value(event).unwrap_or(Value::Null),
                    None => Value::Null,
                }),
            }
        }
        _ => return None,
    })
}
