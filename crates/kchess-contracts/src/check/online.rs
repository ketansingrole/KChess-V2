//! The arguments of online play, challenges, watching, tournaments and speech recognition.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::constants::{
    ARENA_CLOCK_MINUTES, ARENA_DURATIONS, ARENA_INCREMENTS, ARENA_WAIT_MINUTES,
    CORRESPONDENCE_DAYS, TV_CHANNELS,
};
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;
use kchess_domain::variant::{VARIANTS, Variant};

use super::*;
use crate::types::{
    CHALLENGE_COLORS, CHAT_ROOMS, ChallengeColor, ChatRoom, DECLINE_REASONS, DeclineReason,
    NewArena, ONLINE_ACTIONS, OnlineAction, OnlineOptions, TournamentSystem, WatchTarget,
};

/// Some text, every rejection of it `msg`.
fn text_or(value: J, msg: &'static str) -> Result<String, Refusal> {
    decode::<String>(value, plain).map_err(|_| msg.to_owned())
}

/// `n` is one of `options`, which the refusal lists.
fn one_of_numbers(n: f64, options: &[f64], msg: Msg, path: &str) -> Result<(), Refusal> {
    ensure(options.contains(&n), msg, path, || {
        let listed: Vec<String> = options.iter().map(f64::to_string).collect();
        format!("one of {}", listed.join(", "))
    })
}

macro_rules! text_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
        #[serde(transparent)]
        pub struct $name(pub String);
    };
}

text_newtype!(
    /// A full FEN position, at most 100 characters.
    FullFen
);
text_newtype!(
    /// The id of a Lichess arena or Swiss event.
    TournamentId
);
text_newtype!(
    /// A tournament's entry code; empty is none.
    TournamentPassword
);
text_newtype!(
    /// A chat message to send: trimmed, 1 to 140 characters.
    ChatText
);
text_newtype!(
    /// Text to search broadcasts for, trimmed: at most 100 characters.
    BroadcastQuery
);
text_newtype!(
    /// One chunk of base64 16-bit little-endian mono PCM, at most the helper's 1 MiB frame.
    VoiceAudio
);

impl Validate for FullFen {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid position.";
        let text = text_or(value, INVALID)?;
        fen(&text, Some(INVALID), Some(INVALID), "")?;
        Ok(FullFen(text))
    }
}

impl Validate for TournamentId {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid tournament.";
        let text = text_or(value, INVALID)?;
        matches(&text, patterns::lichess_id, Some(INVALID), "", "")?;
        Ok(TournamentId(text))
    }
}

impl Validate for TournamentPassword {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid password.";
        let text = text_or(value, INVALID)?;
        max_chars(&text, 100, Some(INVALID), "")?;
        Ok(TournamentPassword(text))
    }
}

impl Validate for ChatText {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text = text_or(value, "Invalid message.")?;
        let text = js::trim(&text);
        min_chars(text, 1, Some("Type a message first."), "")?;
        let too_long = Some("Chat messages are limited to 140 characters.");
        max_chars(text, 140, too_long, "")?;
        Ok(ChatText(text.to_owned()))
    }
}

impl Validate for BroadcastQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        let text: String = decode(value, plain)?;
        let text = js::trim(&text);
        max_chars(text, 100, None, "")?;
        Ok(BroadcastQuery(text.to_owned()))
    }
}

impl Validate for VoiceAudio {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid audio.";
        let audio = text_or(value, INVALID)?;
        let base64 = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=');
        ensure(
            audio.len().is_multiple_of(4)
                && audio.len() <= (1 << 20) / 3 * 4
                && audio.bytes().all(base64),
            Some(INVALID),
            "",
            || "base64 audio".into(),
        )?;
        Ok(VoiceAudio(audio))
    }
}

/// Users whose presence is asked: trimmed Lichess usernames, at most 50.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct PresenceUsers(pub Vec<String>);

impl Validate for PresenceUsers {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid users.";
        let names: Vec<String> = decode(value, |keys| match keys {
            [] => Some(INVALID),
            _ => Some(USERNAME),
        })?;
        let names = names
            .iter()
            .map(|name| username(name))
            .collect::<Result<Vec<_>, _>>()?;
        max_items(&names, 50, Some(INVALID), "")?;
        Ok(PresenceUsers(names))
    }
}

/// The phrases speech recognition listens for: 1 to 4,096 of at most 200 characters, as the
/// `kchess-voice` helper accepts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct VoiceGrammar(pub Vec<String>);

impl Validate for VoiceGrammar {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid voice grammar.";
        let phrases: Vec<String> = decode(value, plain).map_err(|_| INVALID.to_owned())?;
        within(phrases.len(), 1, 4096, Some(INVALID), "")?;
        for phrase in &phrases {
            max_chars(phrase, 200, Some(INVALID), "")?;
        }
        Ok(VoiceGrammar(phrases))
    }
}

/// The microphone's sample rate in Hz: 8,000 to 192,000.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(transparent)]
pub struct SampleRate(pub f64);

impl Validate for SampleRate {
    fn validate(value: J) -> Result<Self, Refusal> {
        const INVALID: &str = "Invalid sample rate.";
        let rate: f64 = decode(value, plain).map_err(|_| INVALID.to_owned())?;
        ensure(rate.is_finite(), Some(INVALID), "", || {
            "a finite number".into()
        })?;
        within(rate, 8_000.0, 192_000.0, Some(INVALID), "")?;
        Ok(SampleRate(rate))
    }
}

/// The session a `watch` or `watchBroadcast` answered.
pub type WatchSession = Bounded<1, 4_294_967_295>;

/// A tournament's leaderboard page: 1 to 200.
pub type StandingPage = Bounded<1, 200>;

impl Validate for OnlineAction {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, ONLINE_ACTIONS, Some("Invalid game action."))
    }
}

impl Validate for ChatRoom {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, CHAT_ROOMS, Some("Invalid chat room."))
    }
}

impl Validate for DeclineReason {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, DECLINE_REASONS, Some("Invalid reason."))
    }
}

impl Validate for TournamentSystem {
    fn validate(value: J) -> Result<Self, Refusal> {
        let systems: Vec<&str> = TournamentSystem::ALL.iter().map(|s| s.as_str()).collect();
        picked(value, &systems, Some("Invalid tournament."))
    }
}

impl Validate for WatchTarget {
    /// A TV channel KChess lists, or else a game id; anything else is no game to watch.
    fn validate(value: J) -> Result<Self, Refusal> {
        let channels: Vec<&str> = TV_CHANNELS.iter().map(|(key, _)| *key).collect();
        watch_target(value, &channels)
    }
}

/// A game to watch among `channels`: a valid channel wins; otherwise a game id is checked.
pub fn watch_target(value: J, channels: &[&str]) -> Result<WatchTarget, Refusal> {
    let invalid = || "Invalid game to watch.".to_string();
    if !value.is_object() {
        return Err(invalid());
    }
    if let Some(channel) = value.get("channel").and_then(J::str)
        && channels.contains(&channel)
    {
        return Ok(WatchTarget::Channel(channel.to_owned()));
    }
    match value.get("gameId").and_then(J::str) {
        Some(id) => {
            matches(id, patterns::game_id, Some("Invalid game id."), "", "")?;
            Ok(WatchTarget::Game(id.to_owned()))
        }
        None => Err(invalid()),
    }
}

/// The online options as sent: their fields checked one by one, for their messages.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase")]
struct SentOnlineOptions {
    minutes: u64,
    increment: u64,
    color: String,
    rated: bool,
    days: Option<u64>,
    variant: Option<String>,
    fen: Option<String>,
    account: Option<String>,
    target: Option<String>,
}

impl Validate for OnlineOptions {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentOnlineOptions = decode(value, |keys| match keys {
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
        within(sent.minutes, 1, 180, Some("Invalid time control."), "")?;
        within(sent.increment, 0, 180, Some("Invalid increment."), "")?;
        let color = Some("Invalid color.");
        one_of(&sent.color, CHALLENGE_COLORS, color, "")?;
        let color: ChallengeColor = picked(J::Str(sent.color), CHALLENGE_COLORS, color)?;
        if let Some(days) = sent.days {
            let msg = Some("Invalid days per move.");
            one_of_numbers(days as f64, &CORRESPONDENCE_DAYS, msg, "")?;
        }
        let variant = match &sent.variant {
            Some(key) => {
                one_of(key, VARIANTS, Some("Invalid variant."), "")?;
                Variant::from_key(key)
            }
            None => None,
        };
        if let Some(position) = &sent.fen {
            fen(position, None, Some("Invalid position."), "fen")?;
        }
        let account = sent.account.as_deref().map(username).transpose()?;
        let target = match &sent.target {
            Some(target) => {
                let target = js::trim(target);
                let valid = target.is_empty() || patterns::username(target);
                ensure(valid, Some(USERNAME), "", String::new)?;
                (!target.is_empty()).then(|| target.to_owned())
            }
            None => None,
        };
        // Both within 180: whole minutes and seconds.
        let small = |n: u64| u32::try_from(n).map_err(|error| error.to_string());
        Ok(OnlineOptions {
            minutes: small(sent.minutes)?,
            increment: small(sent.increment)?,
            days: sent.days.map(small).transpose()?,
            variant,
            fen: sent.fen,
            color,
            rated: sent.rated,
            target,
            account,
        })
    }
}

/// The arena as sent: its clock, schedule and options, and no other keys.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SentArena {
    name: Option<String>,
    clock_time: f64,
    clock_increment: f64,
    minutes: f64,
    wait_minutes: Option<f64>,
    start_date: Option<f64>,
    variant: String,
    rated: bool,
    password: Option<String>,
    description: Option<String>,
}

impl Validate for NewArena {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentArena = decode(value, |keys| match keys {
            ["clockTime"] => Some("Invalid clock."),
            ["clockIncrement"] => Some("Invalid increment."),
            ["minutes"] => Some("Invalid duration."),
            ["waitMinutes"] => Some("Invalid start."),
            ["variant"] => Some("Invalid variant."),
            _ => None,
        })?;
        let name = match &sent.name {
            Some(name) => {
                let name = js::trim(name);
                let too_long = Some("Arena names are limited to 30 characters.");
                max_chars(name, 30, too_long, "name")?;
                let punctuation = Some("Use letters, numbers and simple punctuation.");
                matches(name, patterns::arena_name, punctuation, "name", "")?;
                Some(name.to_owned())
            }
            None => None,
        };
        let clock = Some("Invalid clock.");
        one_of_numbers(sent.clock_time, ARENA_CLOCK_MINUTES, clock, "clockTime")?;
        let increment = Some("Invalid increment.");
        one_of_numbers(
            sent.clock_increment,
            ARENA_INCREMENTS,
            increment,
            "clockIncrement",
        )?;
        let duration = Some("Invalid duration.");
        one_of_numbers(sent.minutes, ARENA_DURATIONS, duration, "minutes")?;
        if let Some(wait) = sent.wait_minutes {
            let start = Some("Invalid start.");
            one_of_numbers(wait, ARENA_WAIT_MINUTES, start, "waitMinutes")?;
        }
        if let Some(start) = sent.start_date {
            let ok = start.is_finite() && start.fract() == 0.0 && start >= 0.0;
            ensure(ok, None, "startDate", || "a time in milliseconds".into())?;
        }
        one_of(&sent.variant, VARIANTS, Some("Invalid variant."), "variant")?;
        let variant = Variant::from_key(&sent.variant).ok_or("Invalid variant.")?;
        if let Some(password) = &sent.password {
            max_chars(password, 60, None, "password")?;
        }
        if let Some(description) = &sent.description {
            max_chars(description, 2000, None, "description")?;
        }
        Ok(NewArena {
            name,
            clock_time: sent.clock_time,
            clock_increment: sent.clock_increment,
            minutes: sent.minutes,
            wait_minutes: sent.wait_minutes,
            // A whole number of milliseconds, as checked.
            start_date: sent.start_date.map(|start| start as i64),
            variant,
            rated: sent.rated,
            password: sent.password,
            description: sent.description,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_and_names_are_kept_as_sent_or_trimmed() {
        use serde_json::json;
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(checked::<FullFen>(json!(start)), Ok(json!(start)));
        assert_eq!(
            checked::<FullFen>(json!("e4")),
            Err("Invalid position.".into())
        );
        assert_eq!(
            checked::<TournamentId>(json!("abcd1234")),
            Ok(json!("abcd1234"))
        );
        assert_eq!(
            checked::<TournamentId>(json!("abc")),
            Err("Invalid tournament.".into())
        );
        assert_eq!(
            checked::<TournamentPassword>(json!("open")),
            Ok(json!("open"))
        );
        assert_eq!(
            checked::<TournamentPassword>(json!("x".repeat(101))),
            Err("Invalid password.".into())
        );
        assert_eq!(
            checked::<BroadcastQuery>(json!("  world ")),
            Ok(json!("world"))
        );
        assert_eq!(
            checked::<PresenceUsers>(json!([" Alice ", "bob42"])),
            Ok(json!(["Alice", "bob42"]))
        );
        assert_eq!(
            checked::<PresenceUsers>(json!("Alice")),
            Err("Invalid users.".into())
        );
        assert_eq!(checked::<PresenceUsers>(json!([1])), Err(USERNAME.into()));
    }

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    #[test]
    fn a_watch_target_is_a_listed_channel_or_a_game() {
        let channel = obj(vec![("channel", J::Str("rapid".into()))]);
        assert_eq!(
            check::<WatchTarget>(channel),
            Ok(WatchTarget::Channel("rapid".into()))
        );
        let both = obj(vec![
            ("channel", J::Str("nowhere".into())),
            ("gameId", J::Str("abcdefgh".into())),
        ]);
        assert_eq!(
            check::<WatchTarget>(both),
            Ok(WatchTarget::Game("abcdefgh".into()))
        );
        let neither = obj(vec![("channel", J::Str("nowhere".into()))]);
        assert_eq!(
            check::<WatchTarget>(neither),
            Err("Invalid game to watch.".into())
        );
        assert_eq!(
            serde_json::to_value(WatchTarget::Game("abcdefgh".into())).unwrap(),
            serde_json::json!({ "gameId": "abcdefgh" })
        );
    }

    #[test]
    fn online_options_read_as_the_core_takes_them() {
        let options = obj(vec![
            ("minutes", J::Num(10.0)),
            ("increment", J::Num(5.0)),
            ("color", J::Str("white".into())),
            ("rated", J::Bool(false)),
            ("variant", J::Str("chess960".into())),
            ("target", J::Str("  ".into())),
            ("account", J::Str(" Alice ".into())),
        ]);
        let read = check::<OnlineOptions>(options).unwrap();
        assert_eq!((read.minutes, read.increment), (10, 5));
        assert_eq!(read.color, ChallengeColor::White);
        assert_eq!(read.variant, Some(Variant::Chess960));
        assert_eq!(read.target, None);
        assert_eq!(read.account.as_deref(), Some("Alice"));
        let long = obj(vec![
            ("minutes", J::Num(181.0)),
            ("increment", J::Num(0.0)),
            ("color", J::Str("white".into())),
            ("rated", J::Bool(false)),
        ]);
        assert_eq!(
            check::<OnlineOptions>(long),
            Err("Invalid time control.".into())
        );
    }

    #[test]
    fn speech_arguments_keep_their_messages() {
        assert_eq!(
            check::<SampleRate>(J::Num(f64::INFINITY)),
            Err("Invalid sample rate.".into())
        );
        assert_eq!(
            check::<VoiceGrammar>(J::Arr(vec![])),
            Err("Invalid voice grammar.".into())
        );
        assert_eq!(
            check::<ChatText>(J::Str("  ".into())),
            Err("Type a message first.".into())
        );
    }

    #[test]
    fn binary_data_is_not_text() {
        let bytes = J::Bytes {
            len: 2,
            head: b"hi".to_vec(),
        };
        assert_eq!(check::<ChatText>(bytes), Err("Invalid message.".into()));
    }

    #[test]
    fn default_messages_name_the_rejected_value() {
        assert_eq!(
            check::<NewArena>(obj(vec![])),
            Err("missing field `clockTime`".into())
        );
    }
}
