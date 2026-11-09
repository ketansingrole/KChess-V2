//! The online-play session: the account's
//! event stream (challenges and pairings), the game stream of the board, the idle lobby stream,
//! seeks and challenge creation, recovery after a restart, and the moves and actions of the live
//! game.
//!
//! Lichess's streams are authoritative: every state change comes from an event or a reply, and
//! reconnection retains the playing account. Every movement carries an epoch (`cancel` moves it),
//! so a late stream or reply from an old session is dropped. An ambiguous move or action is never
//! replayed: each mutation is sent once.
//!
//! What leaves this module, on the host's `emit`: `online:state`, `online:event`, `online:error`,
//! `online:lobby`, `online:ongoing-changed` (debounced 750 ms), `challenge:received` and
//! `challenges:update` (from the inbox). Payloads are the TypeScript shapes; absent optional
//! fields are left out, as `JSON.stringify` drops `undefined`.
//!
//! Deviations kept on purpose and documented: the stream opener is in this file (the shared
//! client has no open hook or form-body stream), so it calls the same policy and error mapping;
//! bodyless POSTs send an empty form body; the presence cache keeps finished answers only.

pub mod events;
pub mod ongoing;
pub mod requests;
pub mod session;
pub mod state;

pub use state::{
    ChatLine, Lane, OngoingGame, OnlineAction, OnlineOptions, OnlineSession, PresenceReport, Res,
    Resumed, StartResult, UserPresence, js_number,
};

#[cfg(test)]
mod tests {
    use super::events::truncate;
    use super::requests::game_body;
    use super::state::{OnlineOptions, js_number};

    #[test]
    fn js_numbers_print_as_javascript_does() {
        assert_eq!(js_number(600.0), "600");
        assert_eq!(js_number(0.25), "0.25");
        assert_eq!(js_number(0.0), "0");
    }

    #[test]
    fn truncation_counts_utf16_units() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("a😀b", 2), "a");
    }

    #[test]
    fn game_body_follows_the_board_api_rules() {
        let mut options = OnlineOptions {
            minutes: 10.0,
            increment: 0.0,
            days: None,
            variant: Some("chess960".into()),
            fen: None,
            color: "random".into(),
            rated: false,
            target: None,
            account: None,
        };
        assert_eq!(
            game_body(&options),
            vec![
                ("rated", "false".to_string()),
                ("color", "random".to_string()),
                ("variant", "chess960".to_string()),
            ]
        );
        options.variant = Some("standard".into());
        options.fen = Some("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1".into());
        let body = game_body(&options);
        assert!(body.contains(&("variant", "fromPosition".to_string())));
        assert!(body.iter().any(|(key, _)| *key == "fen"));
    }
}
