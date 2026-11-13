use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use kchess_contracts::types::OnlinePhase;
use kchess_domain::variant::Variant;
use serde_json::Value;

use super::state::{Lane, OnlineOptions};
use crate::lichess::client::Auth;

/// One request of a stream: the method, the path, the query, who makes it, and a form body (for
/// the seek).
#[derive(Clone)]
pub(crate) struct StreamRequest {
    pub(crate) method: reqwest::Method,
    pub(crate) path: String,
    pub(crate) query: Vec<(&'static str, String)>,
    pub(crate) auth: Auth,
    pub(crate) form: Option<Vec<(&'static str, String)>>,
}

impl StreamRequest {
    /// A GET stream of `path` as `auth`.
    pub(crate) fn get(path: impl Into<String>, auth: &Auth) -> StreamRequest {
        StreamRequest {
            method: reqwest::Method::GET,
            path: path.into(),
            query: Vec::new(),
            auth: auth.clone(),
            form: None,
        }
    }
}

pub(crate) type Emitter = Arc<dyn Fn(Value) + Send + Sync>;
pub(crate) type Quiet = Arc<dyn Fn(OnlinePhase, Option<&str>) + Send + Sync>;

/// How one `listen` runs (`ListenOptions`), with the session facts it captured at the start.
#[derive(Clone)]
pub(crate) struct Listen {
    pub(crate) lane: Lane,
    pub(crate) reconnect: bool,
    /// Set by the emitter when the game is over: a clean end of stream does not reconnect.
    pub(crate) finished: Option<Arc<AtomicBool>>,
    /// Reports the stream's health instead of the game connection (the idle lobby stream).
    pub(crate) quiet: Option<Quiet>,
    pub(crate) session: u64,
    pub(crate) account: String,
    pub(crate) game_id: String,
    /// What the player is told when Lichess refuses the request (the seek's rules).
    pub(crate) refused: Option<&'static str>,
}

/// The form fields of a seek or challenge.
pub(crate) fn game_body(options: &OnlineOptions) -> Vec<(&'static str, String)> {
    let has_fen = options.fen.as_deref().is_some_and(|fen| !fen.is_empty());
    let variant = if has_fen && options.variant != Some(Variant::Chess960) {
        Some("fromPosition")
    } else {
        options.variant.map(Variant::key)
    };
    let variant = variant.filter(|variant| !variant.is_empty() && *variant != "standard");
    let mut body = vec![
        ("rated", options.rated.to_string()),
        ("color", options.color.as_str().to_string()),
    ];
    if let Some(variant) = variant {
        body.push(("variant", variant.to_string()));
    }
    if let Some(fen) = &options.fen {
        body.push(("fen", fen.clone()));
    }
    body
}

#[cfg(test)]
mod tests {
    use kchess_contracts::types::ChallengeColor;

    use super::*;

    /// The variant field a seek or challenge sends: none for standard chess, the variant's key,
    /// and `fromPosition` for a position unless the game is Chess960 (whose position it is).
    #[test]
    fn the_variant_sent_follows_the_variant_and_the_position() {
        const FEN: &str = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let cases: [(Option<Variant>, Option<&str>, Option<&str>); 7] = [
            (None, None, None),
            (Some(Variant::Standard), None, None),
            (Some(Variant::Atomic), None, Some("atomic")),
            (Some(Variant::Chess960), None, Some("chess960")),
            (Some(Variant::Chess960), Some(FEN), Some("chess960")),
            (Some(Variant::Standard), Some(FEN), Some("fromPosition")),
            (None, Some(""), None),
        ];
        for (variant, fen, sent) in cases {
            let options = OnlineOptions {
                minutes: 5,
                increment: 3,
                days: None,
                variant,
                fen: fen.map(str::to_string),
                color: ChallengeColor::White,
                rated: false,
                target: None,
                account: None,
            };
            let body = game_body(&options);
            let found = body
                .iter()
                .find(|(key, _)| *key == "variant")
                .map(|(_, value)| value.as_str());
            assert_eq!(found, sent, "{variant:?} from {fen:?}");
            assert_eq!(body[0], ("rated", "false".to_string()));
            assert_eq!(body[1], ("color", "white".to_string()));
        }
    }
}
