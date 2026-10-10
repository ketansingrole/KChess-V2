//! Incoming and outgoing challenges (`crates/kchess-node/js/challenges.ts`): the reader of a
//! challenge event from an account's event stream, and the inbox of pending challenges that
//! emits `challenges:update` whenever it changes.
//!
//! The reader keeps the TypeScript schema's rules (valibot): unknown keys pass, an optional key
//! may be absent but not null, lengths count UTF-16 units, and a malformed event is not a
//! challenge at all. The variant mapping comes from `kchess_domain`.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::host::Host;
pub use kchess_domain::records::challenge::{
    ChallengeEvent, ChallengeInfo, PlayerRef, TimeControl, read_challenge_event,
};

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
    use serde_json::{Number, json};
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
