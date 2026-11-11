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
//! `challenges:update` (from the inbox); absent optional fields are left out.
//!
//! Bodyless POSTs send an empty form body; the presence cache keeps finished answers only.

pub mod events;
pub mod ongoing;
pub mod requests;
pub mod session;
pub mod state;

pub use state::{
    ChatLine, Lane, OngoingGame, OnlineAction, OnlineOptions, OnlineSession, PresenceReport,
    Resumed, StartResult, UserPresence, js_number,
};

#[cfg(test)]
mod tests {
    use super::requests::game_body;
    use super::state::OnlineOptions;
    use kchess_contracts::types::ChallengeColor;
    use kchess_domain::variant::Variant;

    #[test]
    fn game_body_follows_the_board_api_rules() {
        let mut options = OnlineOptions {
            minutes: 10,
            increment: 0,
            days: None,
            variant: Some(Variant::Chess960),
            fen: None,
            color: ChallengeColor::Random,
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
        options.variant = Some(Variant::Standard);
        options.fen = Some("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1".into());
        let body = game_body(&options);
        assert!(body.contains(&("variant", "fromPosition".to_string())));
        assert!(body.iter().any(|(key, _)| *key == "fen"));
    }
}
