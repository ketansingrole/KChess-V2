//! The arguments of the notification centre's methods.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::value::J;

use super::*;
use crate::types::{NotificationInput, NotificationLink};

const NOTIFICATION: &str = "Invalid notification.";

/// A kept notification's id: 1 to 64 letters, digits and `-`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct NotificationId(pub String);

impl Validate for NotificationId {
    fn validate(value: J) -> Result<Self, Refusal> {
        let id: String = decode(value, |_| Some(NOTIFICATION))?;
        ensure(identifier(&id), Some(NOTIFICATION), "", String::new)?;
        Ok(NotificationId(id))
    }
}

/// 1 to 64 letters, digits, `-`, `_`, `.`, `:` and `/`: the ids Lichess and KChess give.
fn identifier(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'/'))
}

/// An id inside a link: empty only where `empty` allows it.
fn link_id(text: &str, empty: bool) -> Result<(), Refusal> {
    ensure(
        (empty && text.is_empty()) || identifier(text),
        Some(NOTIFICATION),
        "",
        String::new,
    )
}

impl Validate for NotificationInput {
    fn validate(value: J) -> Result<Self, Refusal> {
        let input: NotificationInput = decode(value, |_| Some(NOTIFICATION))?;
        min_chars(input.title.trim(), 1, Some(NOTIFICATION), "title")?;
        max_chars(&input.title, 200, None, "title")?;
        max_chars(&input.body, 500, None, "body")?;
        if let Some(key) = &input.key {
            link_id(key, false)?;
        }
        match &input.link {
            NotificationLink::None
            | NotificationLink::LiveGame
            | NotificationLink::ComputerGame => {}
            NotificationLink::Challenge { id } => link_id(id, false)?,
            NotificationLink::Correspondence { account, game } => {
                link_id(account, false)?;
                link_id(game, false)?;
            }
            NotificationLink::BroadcastRound { round, game } => {
                link_id(round, false)?;
                link_id(game, true)?;
            }
            NotificationLink::Update { version } => link_id(version, false)?,
        }
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(value: serde_json::Value) -> Result<NotificationInput, Refusal> {
        NotificationInput::validate(J::from(&value))
    }

    #[test]
    fn a_notification_is_bounded_and_its_links_are_ids() {
        let input = check(json!({
            "title": "Your move against Anna",
            "link": { "kind": "correspondence", "account": "maria", "game": "abcd1234" },
        }))
        .expect("valid");
        assert_eq!(input.body, "");
        assert!(!input.read);
        for refused in [
            json!({ "title": " " }),
            json!({ "title": "x".repeat(201) }),
            json!({ "title": "Hi", "body": "x".repeat(501) }),
            json!({ "title": "Hi", "key": "has space" }),
            json!({ "title": "Hi", "link": { "kind": "challenge", "id": "" } }),
            json!({ "title": "Hi", "link": { "kind": "elsewhere" } }),
        ] {
            assert!(check(refused.clone()).is_err(), "{refused}");
        }
        let round = check(json!({
            "title": "Round 3 has started",
            "link": { "kind": "broadcastRound", "round": "AbCd1234" },
        }))
        .expect("valid");
        assert_eq!(
            round.link,
            NotificationLink::BroadcastRound {
                round: "AbCd1234".into(),
                game: String::new()
            }
        );
        assert!(NotificationId::validate(J::from(&json!("a-1"))).is_ok());
        assert!(NotificationId::validate(J::from(&json!(""))).is_err());
    }
}
