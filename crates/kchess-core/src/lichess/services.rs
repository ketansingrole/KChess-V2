//! The Lichess services as one object the core owns: the Lichess service with its client, the
//! online session (with its challenge inbox), the spectator, the position explorer and the cloud
//! evaluation cache.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::accounts::Lichess;
use super::adapters::{DbExplorerLogin, DbLichessStore, DbLookups, DbTokens};
use super::client::{LichessClient, TokenSource, http_client_builder};
use super::lookups::{CloudCache, Endpoints, PositionLookups};
use super::online::OnlineSession;
use super::policy::Policy;
use super::watch::Spectators;
use crate::capabilities::Capabilities;
use crate::host::Host;
use crate::store::Database;
use crate::usage::UsageBatch;

/// The Lichess services of one core.
pub struct Services {
    lichess: Arc<Lichess>,
    online: OnlineSession,
    spectator: Spectators,
    lookups: PositionLookups,
    cloud: CloudCache,
    usage: Arc<UsageBatch>,
    lifetime: CancellationToken,
}

/// What the engines do with a live online game: `stops` ends the computer, the analysis and the
/// review engine; `busy` is set while assistance is blocked (read by the review queue).
pub struct LiveGame {
    pub stops: Arc<dyn Fn() + Send + Sync>,
    pub busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Services {
    /// The services over the core's database, capabilities and host (`host` is the usage-batching
    /// host, so the policy's counters reach `kchess.db` through the batch).
    pub fn new(
        host: Arc<dyn Host>,
        database: Arc<Database>,
        capabilities: Arc<Capabilities>,
        usage: Arc<UsageBatch>,
        lifetime: CancellationToken,
        origin: Option<String>,
        live: LiveGame,
    ) -> Services {
        let LiveGame {
            stops: game_stops,
            busy,
        } = live;
        let http = http_client_builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let meter: Arc<dyn crate::usage::Meter> = usage.clone();
        let policy = Policy::new(Arc::clone(&host), meter, http.clone(), lifetime.clone());
        // A test origin replaces Lichess for every request (`origin` is only set by the tests).
        let (client, endpoints) = match &origin {
            Some(base) => (
                LichessClient::with_base(base, Arc::clone(&policy)),
                Endpoints::at(base),
            ),
            None => (
                LichessClient::new(Arc::clone(&policy)),
                Endpoints::default(),
            ),
        };
        let tokens: Arc<dyn TokenSource> = Arc::new(DbTokens::new(
            Arc::clone(&database),
            Arc::clone(&capabilities),
        ));
        let store = Arc::new(DbLichessStore::new(Arc::clone(&database)));
        let lichess = Arc::new(Lichess::new(
            client,
            Arc::clone(&capabilities),
            store,
            Arc::clone(&tokens),
            Arc::clone(&host),
        ));
        let online = OnlineSession::new(Arc::clone(&lichess));
        let spectator = Spectators::new(Arc::clone(&lichess), host.clone());
        // Before each state is reported: a live game (or the startup check) stops the engines and the
        // watch, and the engines' busy state is taken from the session as it now is.
        {
            let watch = spectator.clone();
            online.set_state_hook(Arc::new(move |live_game, blocked| {
                if live_game {
                    game_stops();
                    watch.stop();
                }
                busy.store(blocked, std::sync::atomic::Ordering::SeqCst);
            }));
        }
        let lookups = PositionLookups::new(
            Arc::clone(&host),
            http,
            endpoints,
            Arc::new(DbLookups::new(Arc::clone(&database))),
            Arc::new(crate::time::now_ms),
        )
        .with_lifetime(lifetime.clone())
        .with_login(Arc::new(DbExplorerLogin::new(
            Arc::clone(&database),
            tokens,
        )));
        Services {
            lichess,
            online,
            spectator,
            lookups,
            cloud: CloudCache::default(),
            usage,
            lifetime,
        }
    }

    /// The Lichess service, for the review queue's analysis fetch.
    pub fn lichess(&self) -> &Arc<Lichess> {
        &self.lichess
    }

    /// The online session, for the engines' busy check and the facade.
    pub fn online(&self) -> &OnlineSession {
        &self.online
    }

    /// The spectator (the watch of a TV channel, a game or a broadcast round).
    pub fn spectator(&self) -> &Spectators {
        &self.spectator
    }

    /// Cancels the Lichess requests and streams in flight when the core closes. The usage still
    /// pending is kept for the core to flush.
    pub fn abort(&self) {
        self.lifetime.cancel();
    }

    pub fn lookups(&self) -> &PositionLookups {
        &self.lookups
    }

    pub fn cloud(&self) -> &CloudCache {
        &self.cloud
    }

    /// Forgets the cloud evaluations and the Lichess session state, once the core closed.
    pub fn reset(&self) {
        self.cloud.clear();
        self.lichess.reset();
    }

    /// Forgets the usage of accounts that were signed out: traffic still in flight
    /// for them is no longer counted, and what is pending is dropped.
    pub fn forget_usage(&self, accounts: &[String]) {
        self.lichess.client().policy().forget_usage(accounts);
        self.usage.forget(accounts);
    }
}
