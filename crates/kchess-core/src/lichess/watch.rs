//! Watching Lichess games: TV channels and the games they show,
//! a game by id, broadcast rounds and the broadcast listings.
//!
//! One watch runs at a time. Starting another stops the last, and frames or states of a stopped
//! session are never reported. Frames go to a `WatchSink` (`HostSink` emits `watch:state`,
//! `watch:frame` and `watch:broadcast` on the host). Lining a TV game up with its delayed export
//! runs in the background and fails quietly: the board works without it.

use kchess_contracts::CoreEvent;
use std::sync::Arc;
use std::time::Duration;

pub use kchess_contracts::types::{
    BroadcastGame, BroadcastRoundPlace, BroadcastRoundRef, BroadcastSummary, BroadcastTourDetail,
    BroadcastUpdate, TvChannel, TvPastGame, WatchClock, WatchFrame, WatchPlayer, WatchState,
    WatchTarget,
};

use crate::host::Host;

mod feed;
mod listings;
mod spectator;

pub use feed::{
    GameDetails, TvGame, align_tv_moves, broadcast_game, display_fen, parse_game_details,
    parse_tv_game,
};
pub use listings::{broadcast_round, broadcast_summaries, broadcast_tour, broadcasts, tv_channels};
pub use spectator::Spectator;

pub use kchess_domain::misc::constants::TV_CHANNELS;

/// How many earlier games a TV channel keeps for the session.
const PAST_GAMES: usize = 6;
/// The longest broadcast PGN read, in UTF-16 units.
const MAX_BROADCAST_PGN: usize = 200_000;
/// The most PGN text the round stream keeps waiting for a game to end.
const MAX_PGN: usize = 2_000_000;

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

    fn emit(&self, event: CoreEvent) {
        match self.connection {
            Some(connection) => self.host.emit_to(connection, event),
            None => self.host.emit(event),
        }
    }
}

impl WatchSink for HostSink {
    fn frame(&self, frame: WatchFrame) {
        self.emit(CoreEvent::WatchFrame(Box::new(frame)));
    }

    fn state(&self, state: WatchState) {
        self.emit(CoreEvent::WatchState(state));
    }

    fn broadcast(&self, update: BroadcastUpdate) {
        self.emit(CoreEvent::WatchBroadcast(update));
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
        let mut watches = crate::tasks::lock(&self.watches);
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

    /// The calling connection's watch only.
    /// With `session`, only that watch: a stop sent before a newer watch began, but handled
    /// after it, must not end the newer one.
    pub fn stop_mine(&self, session: Option<u64>) {
        let connection = crate::connection::current_or_app();
        let spectator = self
            .watches
            .lock()
            .ok()
            .and_then(|watches| watches.get(&connection).cloned());
        if let Some(spectator) = spectator {
            match session {
                Some(session) => spectator.stop_session(session),
                None => spectator.stop(),
            }
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
