//! Watching Lichess games (`crates/kchess-node/js/spectate.ts`): TV channels and the games they show,
//! a game by id, broadcast rounds and the broadcast listings.
//!
//! One watch runs at a time. Starting another stops the last, and frames or states of a stopped
//! session are never reported. Frames go to a `WatchSink` (`HostSink` emits `watch:state`,
//! `watch:frame` and `watch:broadcast` on the host). Lining a TV game up with its delayed export
//! runs in the background and fails quietly: the board works without it.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::host::Host;

mod feed;
mod listings;
mod spectator;

pub use feed::{
    GameDetails, GameSetup, TvGame, align_tv_moves, broadcast_game, display_fen,
    parse_game_details, parse_tv_game, variant_from_lichess,
};
pub use listings::{broadcast_summaries, broadcast_tour, broadcasts, tv_channels};
pub use spectator::Spectator;

/// `TV_CHANNELS` of `crates/kchess-wasm/js/tvChannels.ts`: key and label, in display order.
pub const TV_CHANNELS: [(&str, &str); 16] = [
    ("best", "Top rated"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("ultraBullet", "UltraBullet"),
    ("chess960", "Chess960"),
    ("kingOfTheHill", "King of the Hill"),
    ("threeCheck", "Three-check"),
    ("antichess", "Antichess"),
    ("atomic", "Atomic"),
    ("horde", "Horde"),
    ("racingKings", "Racing Kings"),
    ("crazyhouse", "Crazyhouse"),
    ("bot", "Bots"),
    ("computer", "Computer"),
];

/// `VARIANTS` of the domain: the variants KChess can play.
const VARIANTS: [&str; 8] = [
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
];
/// How many earlier games a TV channel keeps for the session.
const PAST_GAMES: usize = 6;
/// The longest broadcast PGN read, in UTF-16 units (`pgn.slice(0, 200_000)`).
const MAX_BROADCAST_PGN: usize = 200_000;
/// The most PGN text the round stream keeps waiting for a game to end.
const MAX_PGN: usize = 2_000_000;

/* ── Wire shapes ── */

/// A player of a watched game (`WatchPlayer`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WatchPlayer {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
}

/// A time control in seconds (`clock` of `WatchFrame`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct WatchClock {
    pub initial: f64,
    pub increment: f64,
}

/// Connection health of a watch (`WatchState`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WatchState {
    pub session: u64,
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A game a TV channel showed before the current one (`TvPastGame`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TvPastGame {
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    pub fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub orientation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<&'static str>,
}

/// One position of a watched game (`WatchFrame`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchFrame {
    pub session: u64,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    pub game_id: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    pub orientation: &'static str,
    pub fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white_clock: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub black_clock: Option<f64>,
    pub variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<WatchClock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<&'static str>,
    pub finished: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_fen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<Vec<TvPastGame>>,
}

/// A TV channel with who is playing on it (`TvChannel`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TvChannel {
    pub key: &'static str,
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player: Option<WatchPlayer>,
}

/// A broadcast's listing entry (`BroadcastSummary`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players: Option<String>,
    pub tour_id: String,
    pub tour_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round_name: Option<String>,
    pub ongoing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<i64>,
    pub section: &'static str,
}

/// A round of a broadcast tournament (`BroadcastRoundRef`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRoundRef {
    pub id: String,
    pub name: String,
    pub ongoing: bool,
    pub finished: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<i64>,
}

/// A broadcast tournament with its rounds (`BroadcastTourDetail`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastTourDetail {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub rounds: Vec<BroadcastRoundRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_round_id: Option<String>,
}

/// One board of a broadcast round (`BroadcastGame`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastGame {
    pub id: String,
    pub name: String,
    pub white: WatchPlayer,
    pub black: WatchPlayer,
    pub result: String,
    pub start_fen: String,
    pub moves: Vec<String>,
    pub fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white_clock: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub black_clock: Option<f64>,
    pub ongoing: bool,
    pub pgn: String,
}

/// What a broadcast round's feed sent (`BroadcastUpdate`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastUpdate {
    pub session: u64,
    pub round_id: String,
    pub games: Vec<BroadcastGame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where watch frames, states and broadcast updates go.
pub trait WatchSink: Send + Sync {
    fn frame(&self, frame: WatchFrame);
    fn state(&self, state: WatchState);
    fn broadcast(&self, update: BroadcastUpdate);
}

/// The sink of the host: `watch:frame`, `watch:state` and `watch:broadcast` events.
pub struct HostSink {
    host: Arc<dyn Host>,
    /// The client watching (`crate::connection`), or every client.
    connection: Option<u64>,
}

impl HostSink {
    pub fn new(host: Arc<dyn Host>) -> HostSink {
        HostSink {
            host,
            connection: None,
        }
    }

    /// A sink whose events go to one client only.
    pub fn to(host: Arc<dyn Host>, connection: u64) -> HostSink {
        HostSink {
            host,
            connection: Some(connection),
        }
    }

    fn emit(&self, event: &str, payload: Value) {
        match self.connection {
            Some(connection) => self.host.emit_to(connection, event, payload),
            None => self.host.emit(event, payload),
        }
    }
}

impl WatchSink for HostSink {
    fn frame(&self, frame: WatchFrame) {
        self.emit(
            "watch:frame",
            serde_json::to_value(frame).unwrap_or(Value::Null),
        );
    }

    fn state(&self, state: WatchState) {
        self.emit(
            "watch:state",
            serde_json::to_value(state).unwrap_or(Value::Null),
        );
    }

    fn broadcast(&self, update: BroadcastUpdate) {
        self.emit(
            "watch:broadcast",
            serde_json::to_value(update).unwrap_or(Value::Null),
        );
    }
}

/// The watches of every client: one `Spectator` per connection, so two windows watch two
/// channels at once. Each one's frames go to its own client. Cloning shares them.
#[derive(Clone)]
pub struct Spectators {
    lichess: Arc<crate::lichess::accounts::Lichess>,
    host: Arc<dyn Host>,
    watches: Arc<std::sync::Mutex<std::collections::BTreeMap<u64, Spectator>>>,
}

impl Spectators {
    pub fn new(lichess: Arc<crate::lichess::accounts::Lichess>, host: Arc<dyn Host>) -> Spectators {
        Spectators {
            lichess,
            host,
            watches: Arc::default(),
        }
    }

    /// The calling connection's watch, made on first use.
    pub fn mine(&self) -> Spectator {
        let connection = crate::connection::current_or_app();
        let mut watches = self
            .watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        watches
            .entry(connection)
            .or_insert_with(|| {
                Spectator::new(
                    Arc::clone(&self.lichess),
                    Arc::new(HostSink::to(Arc::clone(&self.host), connection)),
                )
            })
            .clone()
    }

    fn all(&self) -> Vec<Spectator> {
        self.watches
            .lock()
            .map(|watches| watches.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Stops every client's watch: a live game, closing.
    pub fn stop(&self) {
        for spectator in self.all() {
            spectator.stop();
        }
    }

    /// `stopWatching`: the calling connection's watch only.
    pub fn stop_mine(&self) {
        let connection = crate::connection::current_or_app();
        let spectator = self
            .watches
            .lock()
            .ok()
            .and_then(|watches| watches.get(&connection).cloned());
        if let Some(spectator) = spectator {
            spectator.stop();
        }
    }

    /// A client went away: its watch stops.
    pub fn forget(&self, connection: u64) {
        let spectator = self
            .watches
            .lock()
            .ok()
            .and_then(|mut watches| watches.remove(&connection));
        if let Some(spectator) = spectator {
            spectator.stop();
        }
    }
}

/// What a watch follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchTarget {
    Channel(String),
    Game(String),
}

/// How a TV game is lined up with its delayed export (`LINE_UP_DELAY_MS`, `LINE_UP_ATTEMPTS`).
#[derive(Clone, Copy, Debug)]
pub struct LineUpTiming {
    pub delay: Duration,
    pub attempts: u32,
}

impl Default for LineUpTiming {
    fn default() -> LineUpTiming {
        LineUpTiming {
            delay: Duration::from_secs(5),
            attempts: 12,
        }
    }
}
