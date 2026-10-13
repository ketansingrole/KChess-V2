use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Number;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::super::accounts::Lichess;
use super::super::challenges::{ChallengeInbox, PlayerRef};
use super::super::client::{Failure, TokenSource};
use crate::error::CoreError;
use crate::host::Host;

/// `MAX_RECONNECTS`: how many times a dropped game or event stream is re-opened without progress.
pub(crate) const MAX_RECONNECTS: u32 = 6;
/// `ongoingChanged` debounce: streams that start or end together are reported once.
pub(crate) const ONGOING_DEBOUNCE: Duration = Duration::from_millis(750);
/// `presenceCache`: a status answer is reused this long (`ttl: 4000`), at most 100 of them.
pub(crate) const PRESENCE_TTL: Duration = Duration::from_millis(4000);
pub(crate) const PRESENCE_MAX: usize = 100;
/// The announced-games set keeps this many ids.
pub(crate) const ANNOUNCED_MAX: usize = 200;
pub(crate) const SEEK_REJECTED: &str = "Lichess rejected this public seek. The Board API allows Rapid and slower for matchmaking (Blitz and faster need a direct challenge). Pick a longer control or add a username.";
pub(crate) const LOST_CONNECTION: &str =
    "Lost connection to Lichess. Check your network and reopen the game.";

/// A failed call or stream, as the session reports it.
pub type Res<T> = std::result::Result<T, Failure>;

/// The session's streams (`lane`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Events,
    Game,
    Seek,
}

impl Lane {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Lane::Events => "events",
            Lane::Game => "game",
            Lane::Seek => "seek",
        }
    }
}

/// `OnlineOptions`: what to play and how; the Board API body of a seek or challenge is built
/// from it (`gameBody`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineOptions {
    /// Ignored for correspondence games (`days`).
    pub minutes: f64,
    pub increment: f64,
    /// Correspondence: days per move instead of a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<f64>,
    /// Lichess variant key; standard when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// A direct challenge from this position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// `random`, `white` or `black`.
    pub color: String,
    pub rated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// What `start` answers: a challenge's id and URL, a seek in progress, or a correspondence game.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correspondence: Option<bool>,
}

/// A game `resume` reattached to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Resumed {
    pub id: String,
    pub account: String,
}

/// A game one of the connected accounts is playing (`OngoingGame`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    pub color: String,
    pub fen: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<Number>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tournament_id: Option<String>,
}

/// A private chat line of the live game (`ChatLine`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: String,
}

/// One user's status (`UserPresence`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPresence {
    pub online: bool,
    pub playing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playing_id: Option<String>,
}

/// `PresenceReport`: statuses by lower-cased username, and the request's round trip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReport {
    pub users: BTreeMap<String, UserPresence>,
    pub latency_ms: u64,
}

/// `OnlineAction`: what can be done to the live game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnlineAction {
    Resign,
    Abort,
    Takeback,
    DeclineTakeback,
    OfferDraw,
    AcceptDraw,
    DeclineDraw,
    ClaimVictory,
    ClaimDraw,
    Berserk,
}

/// `OnlineHooks` of the service: the session reports these to the host through `emit`.
pub(crate) const CHALLENGE_TYPES: [&str; 3] =
    ["challenge", "challengeCanceled", "challengeDeclined"];

/// The session's mutable state. Never held across an await.
pub(crate) struct State {
    pub(crate) controller: Option<Handle>,
    pub(crate) game_controller: Option<Handle>,
    pub(crate) next_handle: u64,
    /// The game whose stream is open, while it lasts.
    pub(crate) live_game: String,
    /// Whether the open game is a correspondence game (another may replace it).
    pub(crate) live_correspondence: bool,
    pub(crate) protected_game: String,
    pub(crate) protected_account: String,
    pub(crate) recovery_unverified: bool,
    pub(crate) current_account: String,
    pub(crate) pending_id: String,
    pub(crate) epoch: u64,
    pub(crate) resume_attempt: u64,
    /// `RequestScope` generation of attachments: a newer attachment supersedes an older one.
    pub(crate) attach_generation: u64,
    pub(crate) attaching_account: String,
    /// Games whose start has been announced, so a reconnect does not repeat it.
    pub(crate) announced: Vec<String>,
    pub(crate) lobby_account: String,
    pub(crate) lobby: Option<(String, CancellationToken)>,
    pub(crate) ongoing_timer: Option<JoinHandle<()>>,
    pub(crate) presence: Vec<(String, Instant, PresenceReport)>,
}

impl State {
    pub(crate) fn new() -> State {
        State {
            controller: None,
            game_controller: None,
            next_handle: 0,
            live_game: String::new(),
            live_correspondence: false,
            protected_game: String::new(),
            protected_account: String::new(),
            recovery_unverified: true,
            current_account: String::new(),
            pending_id: String::new(),
            epoch: 0,
            resume_attempt: 0,
            attach_generation: 0,
            attaching_account: String::new(),
            announced: Vec::new(),
            lobby_account: String::new(),
            lobby: None,
            ongoing_timer: None,
            presence: Vec::new(),
        }
    }
}

/// One stream controller: `id` identifies it (the TypeScript compares controller identity).
#[derive(Clone)]
pub(crate) struct Handle {
    pub(crate) id: u64,
    pub(crate) token: CancellationToken,
}

/// What runs before each state is reported: given whether a live game (or the startup check) is in
/// progress and whether assistance is blocked.
pub(crate) type StateHook = Arc<dyn Fn(bool, bool) + Send + Sync>;

pub(crate) struct Inner {
    pub(crate) lichess: Arc<Lichess>,
    pub(crate) tokens: Arc<dyn TokenSource>,
    pub(crate) http: reqwest::Client,
    pub(crate) host: Arc<dyn Host>,
    pub(crate) challenges: ChallengeInbox,
    pub(crate) state: Mutex<State>,
    /// Run before each state is reported: a live game stops the engines and the watch, and the
    /// engines' busy state is refreshed.
    pub(crate) state_hook: Mutex<Option<StateHook>>,
}

/// An online session. Cheap to clone; clones share the state and the streams.
#[derive(Clone)]
pub struct OnlineSession {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) fn lock(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) fn aborted(message: &str) -> Failure {
    Failure::Core(CoreError::aborted(message))
}

pub(crate) fn core(message: impl Into<String>) -> Failure {
    Failure::Core(CoreError::new(message))
}

/// `Number(x)` as JavaScript writes it into a URL or form: integers without a fraction.
pub fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
