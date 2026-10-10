//! Incoming and outgoing challenges (`core/src/services/challenges.ts`): the reader of a
//! challenge event from an account's event stream, and the inbox of pending challenges that
//! emits `challenges:update` whenever it changes.
//!
//! The reader keeps the TypeScript schema's rules (valibot): unknown keys pass, an optional key
//! may be absent but not null, lengths count UTF-16 units, and a malformed event is not a
//! challenge at all. The variant mapping comes from `kchess_domain`.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::host::Host;

/// `LIVE_TTL_MS`: a real-time challenge Lichess stops reporting is forgotten after this long.
const LIVE_TTL_MS: i64 = 30 * 60_000;
/// `CORRESPONDENCE_TTL_MS`.
const CORRESPONDENCE_TTL_MS: i64 = 14 * 86_400_000;
/// `MAX_CHALLENGES`: the inbox keeps this many; the oldest sighting goes first.
pub const MAX_CHALLENGES: usize = 50;

/// `GAME_ID` of `domain/patterns.ts`: `^[a-zA-Z0-9]{8,12}$`.
pub fn is_game_id(text: &str) -> bool {
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
#[derive(Clone, Debug, PartialEq)]
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
    let playable_variant =
        kchess_domain::online_game::variant_from_lichess(Some(variant_key.as_str()));
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

/// Pending challenges of every connected account, fed by whichever event stream is open. Every
/// change is reported as `challenges:update` with the whole list, newest first.
pub struct ChallengeInbox {
    entries: Mutex<Vec<ChallengeInfo>>,
    host: Arc<dyn Host>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl ChallengeInbox {
    /// `now` is the clock (`Date.now()` in TypeScript); tests pass their own.
    pub fn new(host: Arc<dyn Host>, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> ChallengeInbox {
        ChallengeInbox {
            entries: Mutex::new(Vec::new()),
            host,
            now,
        }
    }

    /// The pending challenges, newest first.
    pub fn list(&self) -> Vec<ChallengeInfo> {
        let mut entries = lock(&self.entries);
        self.prune(&mut entries);
        sorted(&entries)
    }

    pub fn get(&self, id: &str) -> Option<ChallengeInfo> {
        let mut entries = lock(&self.entries);
        self.prune(&mut entries);
        entries.iter().find(|info| info.id == id).cloned()
    }

    /// Apply one event-stream event; returns the challenge when it is new and incoming.
    pub fn ingest(&self, account: &str, raw: &Value) -> Option<ChallengeInfo> {
        let now = (self.now)();
        let event = read_challenge_event(account, raw, now)?;
        let mut entries = lock(&self.entries);
        let position = entries.iter().position(|info| info.id == event.info.id);
        let fresh = if event.kind == "challenge" {
            // Lichess repeats pending challenges whenever the stream reconnects; keep the first
            // sighting (its place and its time).
            let known_time = position.map(|at| entries[at].received_at);
            let mut info = event.info.clone();
            match position {
                Some(at) => {
                    if let Some(time) = known_time {
                        info.received_at = time;
                    }
                    entries[at] = info;
                }
                None => entries.push(info),
            }
            while entries.len() > MAX_CHALLENGES {
                entries.remove(0);
            }
            known_time.is_none() && event.info.direction == "in"
        } else {
            entries.retain(|info| info.id != event.info.id);
            false
        };
        self.emit(&entries);
        fresh.then_some(event.info)
    }

    /// Remove one challenge that was answered here (accepted, declined or withdrawn).
    pub fn remove(&self, id: &str) {
        let mut entries = lock(&self.entries);
        let before = entries.len();
        entries.retain(|info| info.id != id);
        if entries.len() != before {
            self.emit(&entries);
        }
    }

    /// Forget an account's challenges (it was disconnected).
    pub fn forget(&self, account: &str) {
        let mut entries = lock(&self.entries);
        let before = entries.len();
        entries.retain(|info| !info.account.eq_ignore_ascii_case(account));
        if entries.len() != before {
            self.emit(&entries);
        }
    }

    fn prune(&self, entries: &mut Vec<ChallengeInfo>) {
        let now = (self.now)();
        entries.retain(|info| {
            let ttl = match info.time_control {
                TimeControl::Clock { .. } => LIVE_TTL_MS,
                _ => CORRESPONDENCE_TTL_MS,
            };
            now - info.received_at <= ttl
        });
    }

    /// `changed(list())`: the whole list as `challenges:update` carries it.
    fn emit(&self, entries: &[ChallengeInfo]) {
        let payload = serde_json::to_value(sorted(entries)).unwrap_or(Value::Array(Vec::new()));
        self.host.emit("challenges:update", payload);
    }
}

/// Newest first; a stable sort keeps the insertion order of equal times, as JavaScript does.
fn sorted(entries: &[ChallengeInfo]) -> Vec<ChallengeInfo> {
    let mut list = entries.to_vec();
    list.sort_by(|a, b| b.received_at.cmp(&a.received_at));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Level;
    use serde_json::json;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    fn challenge(overrides: Value) -> Value {
        let mut challenge = json!({
            "id": "AbCd1234",
            "status": "created",
            "challenger": { "id": "bob", "name": "Bob", "rating": 1700 },
            "destUser": { "id": "alice", "name": "Alice", "rating": 1650 },
            "variant": { "key": "standard", "name": "Standard" },
            "rated": true,
            "speed": "rapid",
            "timeControl": { "type": "clock", "limit": 600, "increment": 5 },
            "color": "white",
        });
        for (key, value) in overrides.as_object().cloned().unwrap_or_default() {
            challenge[key] = value;
        }
        json!({ "type": "challenge", "challenge": challenge, "compat": { "board": true } })
    }

    #[test]
    fn reads_incoming_and_outgoing_challenges_from_either_accounts_point_of_view() {
        let incoming = read_challenge_event("Alice", &challenge(json!({})), 1000).unwrap();
        assert_eq!(incoming.info.direction, "in");
        assert_eq!(incoming.info.opponent.name, "Bob");
        assert_eq!(incoming.info.opponent.rating, Some(Number::from(1700)));
        assert_eq!(
            incoming.info.time_control,
            TimeControl::Clock {
                limit: Number::from(600),
                increment: Number::from(5)
            }
        );
        assert!(incoming.info.playable);
        assert_eq!(
            read_challenge_event("Bob", &challenge(json!({})), 1000)
                .unwrap()
                .info
                .direction,
            "out"
        );
        assert!(read_challenge_event("Alice", &json!({ "type": "gameStart" }), 0).is_none());
    }

    #[test]
    fn marks_what_kchess_cannot_play() {
        let house = read_challenge_event(
            "Alice",
            &challenge(json!({ "variant": { "key": "crazyhouse", "name": "Crazyhouse" } })),
            0,
        )
        .unwrap()
        .info;
        assert!(!house.playable);
        assert!(house.problem.unwrap().contains("Crazyhouse"));
        let mut bullet = challenge(json!({}));
        bullet["compat"] = json!({ "board": false });
        assert!(
            !read_challenge_event("Alice", &bullet, 0)
                .unwrap()
                .info
                .playable
        );
    }

    #[test]
    fn rejects_malformed_events_and_oversized_names() {
        assert!(read_challenge_event("Alice", &challenge(json!({ "id": "../../x" })), 0).is_none());
        let long = "x".repeat(200);
        assert!(
            read_challenge_event(
                "Alice",
                &challenge(json!({ "challenger": { "id": "b", "name": long } })),
                0
            )
            .is_none()
        );
    }

    #[derive(Default)]
    struct Updates(AtomicUsize);

    impl Host for Updates {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, event: &str, _: Value) {
            assert_eq!(event, "challenges:update");
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn announces_a_challenge_once_removes_it_when_cancelled_and_forgets_stale_ones() {
        let clock = Arc::new(AtomicI64::new(1_000));
        let now = Arc::clone(&clock);
        let host = Arc::new(Updates::default());
        let inbox = ChallengeInbox::new(
            Arc::clone(&host) as Arc<dyn Host>,
            Arc::new(move || now.load(Ordering::SeqCst)),
        );
        assert_eq!(
            inbox
                .ingest("Alice", &challenge(json!({})))
                .map(|info| info.id),
            Some("AbCd1234".to_string())
        );
        // Lichess repeats pending challenges after a reconnect: not new again.
        assert!(inbox.ingest("Alice", &challenge(json!({}))).is_none());
        assert_eq!(inbox.list().len(), 1);
        let cancelled = json!({
            "type": "challengeCanceled",
            "challenge": challenge(json!({}))["challenge"].clone(),
        });
        inbox.ingest("Alice", &cancelled);
        assert!(inbox.list().is_empty());
        inbox.ingest("Alice", &challenge(json!({})));
        clock.fetch_add(31 * 60_000, Ordering::SeqCst);
        assert!(inbox.list().is_empty());
        inbox.ingest(
            "Alice",
            &challenge(json!({ "timeControl": { "type": "correspondence", "daysPerTurn": 3 } })),
        );
        clock.fetch_add(3 * 86_400_000, Ordering::SeqCst);
        assert_eq!(inbox.list().len(), 1);
        inbox.forget("alice");
        assert!(inbox.list().is_empty());
        assert!(host.0.load(Ordering::SeqCst) > 0);
    }
}
