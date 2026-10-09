//! `core/src/domain/onlineEvent.ts`: validation of Lichess board-API events. The schema is the
//! one valibot applied: an object is anything with the keys (arrays included, as valibot reads
//! them), unknown keys pass through, an optional key may be absent but not null, and the event
//! type picks the schema like valibot's `variant`.

use serde_json::{Map, Value};

use super::{Out, arg};
use crate::js;

const UNREADABLE: &str = "Lichess sent an unreadable game event. Reconnect to refresh the board.";
const INVALID_MOVES: &str = "Lichess sent an invalid move list.";

/// The event types the validator knows; other types are ignored by the caller.
const TYPES: &[&str] = &[
    "gameState",
    "gameFull",
    "gameStart",
    "gameFinish",
    "opponentGone",
    "chatLine",
    "challenge",
    "challengeCanceled",
    "challengeDeclined",
];

/// One valibot schema: what a value must be.
enum Check {
    /// A string of at most this many UTF-16 units.
    Str(usize),
    /// A game id: 8 to 12 letters and digits.
    Id,
    /// A finite number.
    Num,
    /// A finite number, not negative.
    Clock,
    Bool,
    /// One of these strings.
    Pick(&'static [&'static str]),
    /// null, or what the inner schema accepts.
    Nullable(&'static Check),
    /// A loose object with these entries.
    Object(&'static [Field]),
}

/// One entry of a loose object: `required` entries must be present; optional ones may be absent.
struct Field {
    key: &'static str,
    required: bool,
    check: Check,
}

const fn req(key: &'static str, check: Check) -> Field {
    Field {
        key,
        required: true,
        check,
    }
}

const fn opt(key: &'static str, check: Check) -> Field {
    Field {
        key,
        required: false,
        check,
    }
}

const PLAYER: &[Field] = &[
    opt("id", Check::Str(usize::MAX)),
    opt("name", Check::Str(usize::MAX)),
    opt("rating", Check::Num),
    opt("title", Check::Nullable(&Check::Str(8))),
    opt("aiLevel", Check::Num),
];

const GAME_OPPONENT: &[Field] = &[
    opt("username", Check::Str(usize::MAX)),
    opt("id", Check::Str(usize::MAX)),
];

const GAME: &[Field] = &[
    req("gameId", Check::Id),
    opt("color", Check::Pick(&["white", "black"])),
    opt("opponent", Check::Object(GAME_OPPONENT)),
];

const EXPIRATION: &[Field] = &[
    req("idleMillis", Check::Clock),
    req("millisToMove", Check::Clock),
];

/// `stateFields`: the game state, with or without its `type`.
const STATE: &[Field] = &[
    req("moves", Check::Str(10_000)),
    req("status", Check::Str(usize::MAX)),
    opt("wtime", Check::Clock),
    opt("btime", Check::Clock),
    opt("winc", Check::Clock),
    opt("binc", Check::Clock),
    opt("winner", Check::Pick(&["white", "black"])),
    opt("wdraw", Check::Bool),
    opt("bdraw", Check::Bool),
    opt("wtakeback", Check::Bool),
    opt("btakeback", Check::Bool),
    opt("expiration", Check::Object(EXPIRATION)),
];

const CLOCK_OBJECT: &[Field] = &[opt("initial", Check::Clock), opt("increment", Check::Clock)];

const FULL: &[Field] = &[
    req("id", Check::Id),
    req("white", Check::Object(PLAYER)),
    req("black", Check::Object(PLAYER)),
    opt("variant", Check::Object(&[req("key", Check::Str(30))])),
    opt("speed", Check::Str(20)),
    opt("initialFen", Check::Str(120)),
    opt("clock", Check::Nullable(&Check::Object(CLOCK_OBJECT))),
    opt("daysPerTurn", Check::Clock),
    opt("tournamentId", Check::Str(20)),
    req("state", Check::Object(STATE)),
];

const CHALLENGE: &[Field] = &[req("challenge", Check::Object(&[req("id", Check::Id)]))];

const GAME_EVENT: &[Field] = &[req("game", Check::Object(GAME))];

const OPPONENT_GONE: &[Field] = &[
    req("gone", Check::Bool),
    opt("claimWinInSeconds", Check::Clock),
];

const CHAT_LINE: &[Field] = &[
    req("username", Check::Str(40)),
    req("text", Check::Str(400)),
    req("room", Check::Str(usize::MAX)),
];

/// The entries an event of this type must have, by its `type`.
fn entries_of_type(kind: &str) -> Option<&'static [Field]> {
    Some(match kind {
        "gameState" => STATE,
        "gameFull" => FULL,
        "gameStart" | "gameFinish" => GAME_EVENT,
        "opponentGone" => OPPONENT_GONE,
        "chatLine" => CHAT_LINE,
        "challenge" | "challengeCanceled" | "challengeDeclined" => CHALLENGE,
        _ => return None,
    })
}

/// valibot's view of an object: an object, or an array with its indices as keys.
fn entries(input: &Value) -> Option<Map<String, Value>> {
    match input {
        Value::Object(map) => Some(map.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| (i.to_string(), item.clone()))
                .collect(),
        ),
        _ => None,
    }
}

/// The loose object `input` must be, with its output (unknown entries kept as they are).
fn object(input: &Value, fields: &[Field]) -> Option<Map<String, Value>> {
    let entries = entries(input)?;
    let mut out = Map::new();
    for field in fields {
        match entries.get(field.key) {
            Some(value) => {
                out.insert(field.key.to_string(), check(value, &field.check)?);
            }
            None if field.required => return None,
            None => {}
        }
    }
    for (key, value) in &entries {
        if !fields.iter().any(|field| field.key == key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
    Some(out)
}

/// The output of `value` under `check`, or None when the schema rejects it.
fn check(value: &Value, schema: &Check) -> Option<Value> {
    let accepted = |ok: bool| ok.then(|| value.clone());
    match schema {
        Check::Str(max) => accepted(
            value
                .as_str()
                .is_some_and(|text| js::utf16_len(text) <= *max),
        ),
        Check::Id => accepted(value.as_str().is_some_and(is_game_id)),
        Check::Num => accepted(value.is_number()),
        Check::Clock => accepted(value.as_f64().is_some_and(|n| n >= 0.0)),
        Check::Bool => accepted(value.is_boolean()),
        Check::Pick(options) => {
            accepted(value.as_str().is_some_and(|text| options.contains(&text)))
        }
        Check::Nullable(inner) => {
            if value.is_null() {
                Some(Value::Null)
            } else {
                check(value, inner)
            }
        }
        Check::Object(fields) => object(value, fields).map(Value::Object),
    }
}

/// `validateOnlineEvent`: the event, or None when it is not one this client reads (no `type`,
/// or an unknown one); an error when a recognized event is malformed.
pub fn validate(raw: &Value) -> Result<Option<Value>, String> {
    let Value::Object(map) = raw else {
        return Ok(None);
    };
    let Some(kind) = map.get("type") else {
        return Ok(None);
    };
    if !TYPES.contains(&js::to_string(Some(kind)).as_str()) {
        return Ok(None);
    }
    let fields = kind
        .as_str()
        .and_then(entries_of_type)
        .ok_or_else(|| UNREADABLE.to_string())?;
    let event = object(raw, fields).ok_or_else(|| UNREADABLE.to_string())?;
    let moves = match kind.as_str() {
        Some("gameState") => event.get("moves"),
        Some("gameFull") => event.get("state").and_then(|state| state.get("moves")),
        _ => None,
    };
    if let Some(moves) = moves
        .and_then(Value::as_str)
        .filter(|moves| !moves.is_empty())
        && !moves_are_valid(moves)
    {
        return Err(INVALID_MOVES.to_string());
    }
    Ok(Some(Value::Object(event)))
}

/// Every space-separated move is UCI or a crazyhouse drop (`P@e4`).
fn moves_are_valid(moves: &str) -> bool {
    let trimmed = js::trim(moves);
    !trimmed.is_empty()
        && trimmed
            .split(js::is_space)
            .filter(|token| !token.is_empty())
            .all(|token| is_uci(token) || is_drop(token))
}

fn is_file(b: u8) -> bool {
    (b'a'..=b'h').contains(&b)
}

fn is_rank(b: u8) -> bool {
    (b'1'..=b'8').contains(&b)
}

/// `UCI_MOVE`: `e2e4`, `e7e8q`.
fn is_uci(token: &str) -> bool {
    let b = token.as_bytes();
    (b.len() == 4 || b.len() == 5)
        && is_file(b[0])
        && is_rank(b[1])
        && is_file(b[2])
        && is_rank(b[3])
        && (b.len() == 4 || b"qrbn".contains(&b[4]))
}

/// `DROP`: `P@e4`.
fn is_drop(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() == 4 && b"PNBRQ".contains(&b[0]) && b[1] == b'@' && is_file(b[2]) && is_rank(b[3])
}

/// `GAME_ID`: 8 to 12 letters and digits.
fn is_game_id(text: &str) -> bool {
    (8..=12).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "validateOnlineEvent" => match arg(args, 0) {
            None => Ok(Value::Null),
            Some(raw) => validate(raw).map(|event| event.unwrap_or(Value::Null)),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn game_state_moves_are_checked_as_uci_or_drops() {
        let state =
            |moves: &str| json!({ "type": "gameState", "moves": moves, "status": "started" });
        assert!(validate(&state("e2e4 e7e5 e7e8q P@e4")).unwrap().is_some());
        assert!(validate(&state("")).unwrap().is_some());
        assert_eq!(validate(&state("e2e4 garbage")).unwrap_err(), INVALID_MOVES);
        assert_eq!(validate(&state("   ")).unwrap_err(), INVALID_MOVES);
    }

    #[test]
    fn unknown_or_untyped_events_are_ignored() {
        assert_eq!(validate(&json!({ "type": "futureEvent" })).unwrap(), None);
        assert_eq!(validate(&json!(["gameState"])).unwrap(), None);
        assert_eq!(validate(&json!({ "moves": "e2e4" })).unwrap(), None);
    }

    #[test]
    fn arrays_read_as_objects_and_optional_keys_reject_null() {
        let full = json!({
            "type": "gameFull",
            "id": "Abcdefgh1",
            "white": [],
            "black": {},
            "state": { "moves": "", "status": "started" },
        });
        let event = validate(&full).unwrap().unwrap();
        assert_eq!(event["white"], json!({}));
        let bad_speed = json!({
            "type": "gameFull",
            "id": "Abcdefgh1",
            "white": {},
            "black": {},
            "speed": null,
            "state": { "moves": "", "status": "started" },
        });
        assert_eq!(validate(&bad_speed).unwrap_err(), UNREADABLE);
    }
}
