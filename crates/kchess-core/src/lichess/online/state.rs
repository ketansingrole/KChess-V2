use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kchess_contracts::types::OnlinePhase;
pub use kchess_contracts::types::{
    ChatLine, OngoingGame, OnlineAction, OnlineOptions, PresenceReport, Resumed, StartResult,
    UserPresence,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::super::accounts::Lichess;
use super::super::challenges::ChallengeInbox;
use crate::host::Host;

/// How many times a dropped game or event stream is re-opened without progress.
pub(crate) const MAX_RECONNECTS: u32 = 6;
/// Streams that start or end together within this time are reported once.
pub(crate) const ONGOING_DEBOUNCE: Duration = Duration::from_millis(750);
/// A status answer is reused this long, at most `PRESENCE_MAX` of them.
pub(crate) const PRESENCE_TTL: Duration = Duration::from_millis(4000);
pub(crate) const PRESENCE_MAX: usize = 100;
pub(crate) const ANNOUNCED_MAX: usize = 200;
pub(crate) const SEEK_REJECTED: &str = "Lichess rejected this public seek. The Board API allows Rapid and slower for matchmaking (Blitz and faster need a direct challenge). Pick a longer control or add a username.";
pub(crate) const LOST_CONNECTION: &str =
    "Lost connection to Lichess. Check your network and reopen the game.";

pub use kchess_contracts::types::Lane;

/// `OnlineHooks` of the service: the session reports these to the host through `emit`.
pub(crate) const CHALLENGE_TYPES: [&str; 3] =
    ["challenge", "challengeCanceled", "challengeDeclined"];

/// The session's mutable state. Never held across an await.
pub(crate) struct State {
    pub(crate) controller: Option<Handle>,
    pub(crate) game_controller: Option<Handle>,
    pub(crate) next_handle: u64,
    /// The game on the board, while its stream lasts. It stays while a real-time seek streams
    /// from a correspondence game: the seek holds `game_controller`, the game stays playable.
    pub(crate) live: Option<LiveGame>,
    /// A move is being sent: another, from any window, is refused until it is answered.
    pub(crate) move_in_flight: bool,
    pub(crate) protected_game: String,
    pub(crate) protected_account: String,
    pub(crate) recovery_unverified: bool,
    pub(crate) current_account: String,
    /// The challenge sent and not yet answered, cancelled with the session.
    pub(crate) pending_challenge: Option<String>,
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

/// What a game Lichess says started (`gameStart`) does to the board (`State::started`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Started {
    OnBoard,
    Opens,
    ListedOnly,
}

/// Whether a state reported in `phase` for `game_id` is a live game (or the startup check) in
/// progress: what stops the engines and the watch.
pub(crate) fn reports_live_game(phase: OnlinePhase, game_id: &str) -> bool {
    phase == OnlinePhase::Checking || (!game_id.is_empty() && phase != OnlinePhase::Idle)
}

/// The game on the board.
pub(crate) struct LiveGame {
    pub(crate) id: String,
    /// A correspondence game: another, real-time, may replace it.
    pub(crate) correspondence: bool,
}

impl State {
    /// Whether `id` is the game on the board.
    pub(crate) fn is_live(&self, id: &str) -> bool {
        self.live.as_ref().is_some_and(|game| game.id == id)
    }

    /// The board game's id, or empty with none (as the window's state reports it).
    pub(crate) fn live_id(&self) -> &str {
        self.live.as_ref().map_or("", |game| game.id.as_str())
    }

    /// A real-time game holds the board: nothing else may take it.
    pub(crate) fn realtime_live(&self) -> bool {
        self.live.as_ref().is_some_and(|game| !game.correspondence)
    }

    /// Game `id` left the board (it ended, or another took its place).
    pub(crate) fn end_live(&mut self, id: &str) {
        if self.is_live(id) {
            self.live = None;
        }
    }

    /// The game id a lane's state reports: the board's game on the game lane, none elsewhere.
    pub(crate) fn lane_game(&self, lane: Lane) -> &str {
        if lane == Lane::Game {
            self.live_id()
        } else {
            ""
        }
    }

    /// What a game Lichess says started (`gameStart`) does to the board: nothing when it is
    /// there already; it is only listed when it is a correspondence game (opened when asked
    /// for) or a real-time game holds the board; otherwise it opens.
    pub(crate) fn started(&self, id: &str, correspondence: bool) -> Started {
        if id == self.live_id() {
            Started::OnBoard
        } else if correspondence || self.realtime_live() {
            Started::ListedOnly
        } else {
            Started::Opens
        }
    }

    /// Whether a game's event (`gameFinish`) concerns this session: the board's game, or the
    /// protected one.
    pub(crate) fn concerns(&self, id: &str) -> bool {
        self.is_live(id) || self.protected_game == id
    }

    /// Game `id`'s stream (`handle_id`) ended: the game leaves the board when it
    /// finished and the stream is still the board's; a stream that only dropped keeps it.
    pub(crate) fn stream_ended(&mut self, id: &str, handle_id: u64, finished: bool) {
        if finished && super::ongoing::finished_check(self, handle_id) {
            self.end_live(id);
        }
    }

    pub(crate) fn new() -> State {
        State {
            controller: None,
            game_controller: None,
            next_handle: 0,
            live: None,
            move_in_flight: false,
            protected_game: String::new(),
            protected_account: String::new(),
            recovery_unverified: true,
            current_account: String::new(),
            pending_challenge: None,
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

/// One stream controller: `id` identifies it.
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

pub(crate) use crate::tasks::lock;

pub use kchess_domain::js::number_to_string as js_number;

#[cfg(test)]
mod tests {
    use super::{Handle, LiveGame, Started, State, reports_live_game};
    use crate::lichess::online::state::Lane;
    use kchess_contracts::types::OnlinePhase;
    use tokio_util::sync::CancellationToken;

    fn with_live(id: &str, correspondence: bool) -> State {
        let mut st = State::new();
        st.live = Some(LiveGame {
            id: id.into(),
            correspondence,
        });
        st
    }

    /// The startup check and any state of a known game but idle stop the engines and the watch;
    /// the account's own stream, with no game, does not.
    #[test]
    fn a_live_game_is_the_check_or_a_known_game_not_idle() {
        assert!(reports_live_game(OnlinePhase::Checking, ""));
        assert!(reports_live_game(OnlinePhase::Connected, "AbCd1234"));
        assert!(!reports_live_game(OnlinePhase::Connected, ""));
        assert!(!reports_live_game(OnlinePhase::Idle, "AbCd1234"));
    }

    #[test]
    fn only_the_game_lane_reports_the_boards_game() {
        let st = with_live("AbCd1234", false);
        assert_eq!(st.lane_game(Lane::Game), "AbCd1234");
        assert_eq!(st.lane_game(Lane::Events), "");
        assert_eq!(State::new().lane_game(Lane::Game), "");
    }

    /// A started game opens on an empty board, or over a correspondence game; it is only listed
    /// when it is a correspondence game or a real-time game holds the board.
    #[test]
    fn a_started_game_opens_unless_the_board_is_taken_or_it_is_correspondence() {
        let empty = State::new();
        assert_eq!(empty.started("Other123", false), Started::Opens);
        assert_eq!(empty.started("Corr1234", true), Started::ListedOnly);
        let realtime = with_live("AbCd1234", false);
        assert_eq!(realtime.started("AbCd1234", false), Started::OnBoard);
        assert_eq!(realtime.started("Other123", false), Started::ListedOnly);
        let correspondence = with_live("Corr1234", true);
        assert_eq!(correspondence.started("Other123", false), Started::Opens);
    }

    #[test]
    fn a_game_event_concerns_the_boards_game_or_the_protected_one() {
        let mut st = with_live("AbCd1234", false);
        st.protected_game = "Prot1234".into();
        assert!(st.concerns("AbCd1234"));
        assert!(st.concerns("Prot1234"));
        assert!(!st.concerns("Other123"));
    }

    /// Only the board's own stream ending a finished game takes it off the board; one that only
    /// dropped, or an older stream, leaves it.
    #[test]
    fn a_game_leaves_the_board_when_its_own_stream_ends_it() {
        let mut st = with_live("AbCd1234", false);
        st.game_controller = Some(Handle {
            id: 7,
            token: CancellationToken::new(),
        });
        st.stream_ended("AbCd1234", 7, false);
        assert!(st.is_live("AbCd1234"), "a dropped stream keeps the game");
        st.stream_ended("AbCd1234", 8, true);
        assert!(st.is_live("AbCd1234"), "an older stream keeps it");
        st.stream_ended("AbCd1234", 7, true);
        assert!(st.live.is_none());
    }
}
