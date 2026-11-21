//! The spectator: one watch at a time, its streams and what it reports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::feed::{fen_ply, player_of, position_key, uci_or_none};
use super::{
    BroadcastGame, BroadcastUpdate, LineUpTiming, MAX_PGN, PAST_GAMES, TvGame, TvMoves, TvPastGame,
    WatchFrame, WatchSink, WatchState, WatchTarget, broadcast_game, display_fen,
    parse_game_details, parse_tv_game, tv_moves, tv_moves_since,
};
use crate::host::Level;
use crate::lichess::accounts::{Lichess, path_segment};
use crate::lichess::client::{Auth, Lane};
use crate::lichess::decode::{Loose, read};
use crate::lichess::stream::STALLED;
use kchess_contracts::error::{CoreError, Result as CoreResult};
use kchess_contracts::types::{UsageKind, WatchPhase, WatchSource};
use kchess_domain::color::Color;

/// A TV feed record, `{ t, d }`: a featured game or a move. Other kinds, and records whose
/// `d` is not an object, are ignored; each field is read when it has the right type.
#[derive(Deserialize)]
#[serde(tag = "t", content = "d")]
enum TvRecord {
    #[serde(rename = "featured")]
    Featured(TvFeatured),
    #[serde(rename = "fen")]
    Fen(FeedMove),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct TvFeatured {
    #[serde(default)]
    players: Loose<Vec<Value>>,
    #[serde(default)]
    fen: Loose<String>,
    #[serde(default)]
    id: Loose<String>,
    #[serde(default)]
    orientation: Loose<String>,
}

/// A move of a feed: the position, the move and the clocks.
#[derive(Deserialize)]
struct FeedMove {
    #[serde(default)]
    fen: Loose<String>,
    #[serde(default)]
    lm: Option<Value>,
    #[serde(default)]
    wc: Loose<f64>,
    #[serde(default)]
    bc: Loose<f64>,
}

#[derive(Default, Deserialize)]
struct Named {
    #[serde(default)]
    name: Loose<String>,
}

#[derive(Default, Deserialize)]
struct Keyed {
    #[serde(default)]
    key: Loose<String>,
}

/// A record of a game's stream: its description (with an `id`) or a move.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameRecord {
    #[serde(default)]
    fen: Loose<String>,
    /// Where the game started; Lichess's description of a game carries this, not its `fen`.
    #[serde(default)]
    initial_fen: Loose<String>,
    #[serde(default)]
    id: Loose<String>,
    #[serde(default)]
    status: Loose<Named>,
    #[serde(default)]
    players: Loose<Map<String, Value>>,
    #[serde(default)]
    winner: Loose<String>,
    #[serde(default)]
    variant: Loose<Keyed>,
    #[serde(default)]
    last_move: Option<Value>,
    #[serde(default)]
    lm: Option<Value>,
    #[serde(default)]
    wc: Loose<f64>,
    #[serde(default)]
    bc: Loose<f64>,
}

/// The standard start, for a game described without a position.
const STANDARD_START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// What a viewer is told when a feed's task panicked (the panic hook logged it).
const FEED_PANICKED: &str = "The feed stopped after an internal error. Retry to reconnect.";

/* ── Spectator ── */

#[derive(Default)]
struct Control {
    session: u64,
    /// The running session's token, cancelled whenever `session` moves on, in the same lock: a
    /// cancelled token is a session no longer current.
    cancel: Option<CancellationToken>,
    /// Earlier games per TV channel, kept for the app's lifetime.
    past: HashMap<String, Vec<TvPastGame>>,
}

/// What a viewer is told when the featured game is in a variant KChess does not play.
const UNPLAYABLE: &str = "KChess does not play this variant.";

/// The TV game being followed, with what the feed has shown of it.
#[derive(Default)]
struct TvLive {
    current: Option<WatchFrame>,
    follow: Option<Arc<TvFollow>>,
}

/// What the feed has shown of the featured game, to line it up with Lichess's export.
#[derive(Default)]
struct TvFollow {
    track: Mutex<TvTrack>,
    /// Rung by each move the feed sends while the line-up runs: the export has one more ply.
    moved: Notify,
}

#[derive(Default)]
struct TvTrack {
    /// The positions the feed showed, as `position_key`s.
    feed: Vec<String>,
    /// `raw[i]` is the FEN the feed sent for `feed[i]`, with its move number and clock.
    raw: Vec<String>,
    /// `live[i]` is the move from `feed[i]` to `feed[i + 1]`, when the feed named it.
    live: Vec<Option<String>>,
    /// A line-up task is running for this game.
    lining_up: bool,
    /// The export is in a variant KChess does not play: there are no moves to line up.
    unplayable: bool,
}

/// A line-up worked out while the feed had shown `seen` positions.
struct Lined {
    seen: usize,
    moves: TvMoves,
}

impl Lined {
    fn caught_up(self, track: &TvTrack) -> TvMoves {
        tv_moves_since(self.moves, self.seen, &track.raw, &track.live)
    }
}

struct Inner {
    lichess: Arc<Lichess>,
    sink: Arc<dyn WatchSink>,
    timing: LineUpTiming,
    control: Mutex<Control>,
}

use crate::tasks::lock;

/// Watches one game or TV channel, or a broadcast round, at a time.
#[derive(Clone)]
pub struct Spectator {
    inner: Arc<Inner>,
}

impl Spectator {
    pub fn new(lichess: Arc<Lichess>, sink: Arc<dyn WatchSink>) -> Spectator {
        Spectator::with_timing(lichess, sink, LineUpTiming::default())
    }

    pub fn with_timing(
        lichess: Arc<Lichess>,
        sink: Arc<dyn WatchSink>,
        timing: LineUpTiming,
    ) -> Spectator {
        Spectator {
            inner: Arc::new(Inner {
                lichess,
                sink,
                timing,
                control: Mutex::new(Control::default()),
            }),
        }
    }

    /// Stops the running watch; its frames and states are no longer reported.
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// Ends `session` if it is still the running one; a newer watch carries on.
    pub fn stop_session(&self, session: u64) {
        let mut control = lock(&self.inner.control);
        if control.session != session {
            return;
        }
        control.session += 1;
        if let Some(running) = control.cancel.take() {
            running.cancel();
        }
    }

    /// Watches a TV channel or a game; returns the session number its frames carry.
    pub fn watch(&self, target: WatchTarget) -> u64 {
        let (session, token) = self.inner.begin();
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            inner.report(session, &token, WatchPhase::Connecting, None);
            let followed = async {
                match &target {
                    WatchTarget::Channel(channel) => inner
                        .follow_tv(channel, session, &token)
                        .await
                        .map(|()| false),
                    WatchTarget::Game(id) => inner.follow_game(id, session, &token).await,
                }
            };
            let label = match &target {
                WatchTarget::Channel(channel) | WatchTarget::Game(channel) => channel,
            };
            let what = format!("Watch failed: {label}");
            match inner.outcome(&token, &what, followed).await {
                None => {}
                // Lichess closes a game's stream once the game is over: nothing to reconnect to.
                Some(Ok(true)) => inner.report(session, &token, WatchPhase::Ended, None),
                Some(Ok(false)) => inner.report(
                    session,
                    &token,
                    WatchPhase::Ended,
                    Some("The feed ended. Retry to reconnect."),
                ),
                Some(Err(cause)) => {
                    let kind = cause
                        .kind
                        .unwrap_or(kchess_contracts::error::ErrorKind::Offline);
                    let message = said_or(cause, "The feed disconnected.");
                    inner.report_failure(session, &token, &message, kind);
                }
            }
        });
        session
    }

    /// Follows a broadcast round's live PGN; returns the session number.
    pub fn watch_round(&self, round_id: &str) -> u64 {
        let (session, token) = self.inner.begin();
        let inner = Arc::clone(&self.inner);
        let round_id = round_id.to_string();
        tokio::spawn(async move {
            let followed = inner.follow_round(&round_id, session, &token);
            let what = format!("Broadcast feed failed: {round_id}");
            let Some(outcome) = inner.outcome(&token, &what, followed).await else {
                return;
            };
            if inner.is_current(session) && !token.is_cancelled() {
                inner.sink.broadcast(BroadcastUpdate {
                    session,
                    round_id: round_id.clone(),
                    games: Vec::new(),
                    ended: Some(true),
                    error: outcome
                        .err()
                        .map(|cause| said_or(cause, "The broadcast feed stopped.")),
                });
            }
        });
        session
    }
}

/// A failure's message, or `fallback` when it says nothing.
fn said_or(cause: CoreError, fallback: &str) -> String {
    if cause.message.is_empty() {
        fallback.to_string()
    } else {
        cause.message
    }
}

impl Inner {
    /// How a feed ended: a panic as a failure, so the viewer is offered a retry; a stop or a
    /// newer watch as `None`, since nothing failed; and a failure logged as `what`.
    async fn outcome<T>(
        &self,
        token: &CancellationToken,
        what: &str,
        followed: impl std::future::Future<Output = CoreResult<T>>,
    ) -> Option<CoreResult<T>> {
        let result = crate::tasks::caught(followed)
            .await
            .unwrap_or_else(|_| Err(CoreError::new(FEED_PANICKED)));
        match result {
            Err(_) if token.is_cancelled() => None,
            Err(cause) => {
                self.host_log(Level::Warn, &format!("{what} {}", cause.message));
                Some(Err(cause))
            }
            done => Some(done),
        }
    }

    /// Who the watch's requests are made as: nobody, counted as watching.
    fn watcher(&self) -> Auth {
        self.lichess.anonymous(UsageKind::Watch)
    }

    /// Starts a session: stops the running one and returns the new session and its token.
    fn begin(&self) -> (u64, CancellationToken) {
        let mut control = lock(&self.control);
        if let Some(previous) = control.cancel.take() {
            previous.cancel();
        }
        control.session += 1;
        let token = CancellationToken::new();
        control.cancel = Some(token.clone());
        (control.session, token)
    }

    /// Ends the running session, if any: its frames and states stop being reported.
    fn stop(&self) {
        let mut control = lock(&self.control);
        control.session += 1;
        if let Some(running) = control.cancel.take() {
            running.cancel();
        }
    }

    fn is_current(&self, session: u64) -> bool {
        lock(&self.control).session == session
    }

    fn host_log(&self, level: Level, message: &str) {
        self.lichess.host().log(level, "spectate", message);
    }

    fn report(
        &self,
        session: u64,
        token: &CancellationToken,
        phase: WatchPhase,
        message: Option<&str>,
    ) {
        if !token.is_cancelled() && self.is_current(session) {
            self.sink.state(WatchState {
                session,
                phase,
                message: message.map(str::to_string),
                kind: None,
            });
        }
    }

    /// The feed failed: its state says what kind of failure, for the viewer's next step.
    fn report_failure(
        &self,
        session: u64,
        token: &CancellationToken,
        message: &str,
        kind: kchess_contracts::error::ErrorKind,
    ) {
        if !token.is_cancelled() && self.is_current(session) {
            self.sink.state(WatchState {
                session,
                phase: WatchPhase::Error,
                message: Some(message.to_string()),
                kind: Some(kind),
            });
        }
    }

    /// The TV channel's past games, as frames carry them.
    fn previous_of(&self, channel: &str) -> Option<Vec<TvPastGame>> {
        lock(&self.control).past.get(channel).cloned()
    }

    /// Emits a TV frame with the channel's current history.
    fn emit_tv(&self, channel: &str, mut frame: WatchFrame) {
        frame.previous = self.previous_of(channel);
        self.sink.frame(frame);
    }

    async fn follow_tv(
        self: &Arc<Self>,
        channel: &str,
        session: u64,
        token: &CancellationToken,
    ) -> CoreResult<()> {
        let path = format!("/api/tv/{}/feed", path_segment(channel));
        let live = Arc::new(Mutex::new(TvLive::default()));
        loop {
            let live_for_lines = Arc::clone(&live);
            let token_for_lines = token.clone();
            let inner = Arc::clone(self);
            let channel_owned = channel.to_string();
            let followed = self
                .lichess
                .client()
                .get(&path, &self.watcher())
                .lane(Lane::Live)
                .cancel(token)
                .lines(
                    || {},
                    move |line| {
                        if token_for_lines.is_cancelled() || !inner.is_current(session) {
                            return Ok(());
                        }
                        // A malformed record ends the feed as it arrives.
                        inner.handle_tv_line(
                            &channel_owned,
                            session,
                            &token_for_lines,
                            &live_for_lines,
                            line,
                        )?;
                        Ok(())
                    },
                )
                .await;
            match followed {
                // The TV feed has no keep-alive: it is silent while a player thinks. A new
                // request answers with the featured game at once.
                Err(cause)
                    if cause.message == STALLED
                        && !token.is_cancelled()
                        && self.is_current(session) => {}
                other => return other,
            }
        }
    }

    /// One line of a TV feed: a featured game (`featured`) or a move (`fen`).
    fn handle_tv_line(
        self: &Arc<Self>,
        channel: &str,
        session: u64,
        token: &CancellationToken,
        live: &Arc<Mutex<TvLive>>,
        line: &str,
    ) -> CoreResult<()> {
        let message: Value = serde_json::from_str(line).map_err(|cause| {
            CoreError::new(format!("Lichess sent an unreadable TV record: {cause}"))
        })?;
        let fen_of = |fen: &Loose<String>| display_fen(fen.0.as_deref().unwrap_or(""));
        match read::<TvRecord>(&message).unwrap_or(TvRecord::Other) {
            TvRecord::Featured(data) => {
                let players = data.players.0.unwrap_or_default();
                let side = |color: &str| {
                    players
                        .iter()
                        .find(|player| player.get("color").and_then(Value::as_str) == Some(color))
                        .cloned()
                };
                let Some(fen) = fen_of(&data.fen) else {
                    return Ok(());
                };
                let raw = data.fen.0.clone().unwrap_or_default();
                let Some(id) = data.id.0.as_deref() else {
                    return Ok(());
                };
                let game_id = kchess_domain::js::slice_utf16(id, 12).to_string();
                let white = side("white");
                let black = side("black");
                let seconds = |player: &Option<Value>| {
                    player
                        .as_ref()
                        .and_then(|p| p.get("seconds"))
                        .and_then(Value::as_f64)
                };
                let (white_clock, black_clock) = (seconds(&white), seconds(&black));
                let mut state = lock(live);
                // The feed has no keep-alive, so it reconnects after a long think and sends its
                // game again: the game goes on, with its moves and details.
                if let (Some(mut frame), Some(follow)) =
                    (state.current.clone(), state.follow.clone())
                    && frame.game_id == game_id
                {
                    let (moved, missed) = {
                        let mut track = lock(&follow.track);
                        let key = position_key(&fen);
                        let moved = track.feed.last() != Some(&key);
                        let missed = track
                            .raw
                            .last()
                            .and_then(|before| Some(fen_ply(&raw)?.checked_sub(fen_ply(before)?)?));
                        if moved {
                            track.feed.push(key);
                            track.raw.push(raw);
                            track.live.push(None);
                        }
                        (moved, missed)
                    };
                    frame.white_clock = white_clock.or(frame.white_clock);
                    frame.black_clock = black_clock.or(frame.black_clock);
                    if moved {
                        // The moves made while it was away are not named: the list stops short
                        // of them until the export names them.
                        frame.fen = fen;
                        frame.last_move = None;
                        match missed.filter(|missed| *missed > 0) {
                            Some(missed) if frame.moves.is_some() => {
                                frame.moves_behind = Some(frame.moves_behind.unwrap_or(0) + missed);
                            }
                            _ => {
                                frame.moves = None;
                                frame.moves_behind = None;
                            }
                        }
                    }
                    state.current = Some(frame.clone());
                    self.report_connected(session);
                    self.emit_tv(channel, frame);
                    drop(state);
                    if moved {
                        self.line_up(channel, live, &follow, &game_id, session, token);
                    }
                    return Ok(());
                }
                if let Some(previous) = state.current.as_ref()
                    && previous.game_id != game_id
                {
                    let previous = previous.clone();
                    self.remember(channel, &previous, token);
                }
                let frame = WatchFrame {
                    session,
                    source: WatchSource::Tv,
                    channel: Some(channel.to_string()),
                    game_id: game_id.clone(),
                    white: player_of(white.as_ref()),
                    black: player_of(black.as_ref()),
                    orientation: if data.orientation.0.as_deref() == Some("black") {
                        Color::Black
                    } else {
                        Color::White
                    },
                    fen: fen.clone(),
                    last_move: None,
                    white_clock,
                    black_clock,
                    // A variant's channel shows that variant; any other (a speed, the best
                    // players, bots) shows standard chess until the export says otherwise.
                    variant: kchess_domain::variant::Variant::from_key(channel)
                        .unwrap_or_default()
                        .key()
                        .to_string(),
                    speed: None,
                    rated: None,
                    clock: None,
                    status: None,
                    winner: None,
                    finished: false,
                    start_fen: None,
                    moves: None,
                    moves_behind: None,
                    moves_error: None,
                    previous: None,
                };
                let follow = Arc::new(TvFollow::default());
                {
                    let mut track = lock(&follow.track);
                    track.feed.push(position_key(&fen));
                    track.raw.push(raw);
                }
                state.current = Some(frame.clone());
                state.follow = Some(Arc::clone(&follow));
                self.report_connected(session);
                self.emit_tv(channel, frame);
                drop(state);
                self.line_up(channel, live, &follow, &game_id, session, token);
                Ok(())
            }
            TvRecord::Fen(data) => {
                let Some(fen) = fen_of(&data.fen) else {
                    return Ok(());
                };
                let raw = data.fen.0.clone().unwrap_or_default();
                let last_move = uci_or_none(data.lm.as_ref());
                let mut state = lock(live);
                let (Some(current), Some(follow)) = (state.current.clone(), state.follow.clone())
                else {
                    return Ok(());
                };
                {
                    let mut track = lock(&follow.track);
                    track.feed.push(position_key(&fen));
                    track.raw.push(raw);
                    track.live.push(last_move.clone());
                }
                let behind = current.moves_behind.unwrap_or(0);
                // A move follows the list when it reaches the board and the feed named the move;
                // otherwise the list stops one more ply short, never takes a wrong one.
                let (moves, moves_behind) = match (current.moves.clone(), last_move.as_ref()) {
                    (Some(mut moves), Some(last)) if behind == 0 => {
                        moves.push(last.clone());
                        (Some(moves), None)
                    }
                    (Some(moves), _) => (Some(moves), Some(behind + 1)),
                    (None, _) => (None, None),
                };
                let frame = WatchFrame {
                    fen,
                    last_move: last_move.clone(),
                    white_clock: data.wc.0.or(current.white_clock),
                    black_clock: data.bc.0.or(current.black_clock),
                    moves,
                    moves_behind,
                    ..current
                };
                let game_id = frame.game_id.clone();
                let lined_up = frame.moves.is_some() && frame.moves_behind.is_none();
                state.current = Some(frame.clone());
                // Sent under the lock, so a line-up's frame never overtakes a newer move.
                self.report_connected(session);
                self.emit_tv(channel, frame);
                drop(state);
                if !lined_up {
                    self.line_up(channel, live, &follow, &game_id, session, token);
                }
                Ok(())
            }
            TvRecord::Other => Ok(()),
        }
    }

    fn report_connected(&self, session: u64) {
        self.sink.state(WatchState {
            session,
            phase: WatchPhase::Connected,
            message: None,
            kind: None,
        });
    }

    /// Lines the featured game up with Lichess's export, which leaves out an ongoing game's last
    /// three moves: it reaches a position the feed showed only three moves after the feed
    /// started (or reconnected), however long those take. A running line-up tries again now.
    fn line_up(
        self: &Arc<Self>,
        channel: &str,
        live: &Arc<Mutex<TvLive>>,
        follow: &Arc<TvFollow>,
        game_id: &str,
        session: u64,
        token: &CancellationToken,
    ) {
        {
            let mut track = lock(&follow.track);
            if track.unplayable {
                return;
            }
            if track.lining_up {
                drop(track);
                follow.moved.notify_one();
                return;
            }
            track.lining_up = true;
        }
        let inner = Arc::clone(self);
        let (live, follow, token) = (Arc::clone(live), Arc::clone(follow), token.clone());
        let (channel, game_id) = (channel.to_string(), game_id.to_string());
        tokio::spawn(async move {
            inner
                .lining_up(&channel, &live, &follow, &game_id, session, &token)
                .await;
        });
    }

    /// Fetches the export until it lines up: at once after each move the feed sends, and every
    /// `timing.delay` for `timing.attempts` tries after the last one. A quiet feed ends the
    /// tries; its next move starts them again (`line_up`).
    async fn lining_up(
        self: &Arc<Self>,
        channel: &str,
        live: &Arc<Mutex<TvLive>>,
        follow: &Arc<TvFollow>,
        game_id: &str,
        session: u64,
        token: &CancellationToken,
    ) {
        let mut left = self.timing.attempts;
        loop {
            if token.is_cancelled() || !self.is_current(session) {
                return;
            }
            let seen = lock(&follow.track).feed.len();
            let fetched = self.fetch_tv_game(game_id, token).await;
            if token.is_cancelled() || !self.is_current(session) {
                return;
            }
            if let Err(cause) = &fetched {
                self.lichess.host().log(
                    Level::Debug,
                    "spectate",
                    &format!("TV lineup fetch failed: {game_id} {}", cause.message),
                );
            }
            let lined = match &fetched {
                Ok(game) => self.line_up_export(game, follow).await,
                Err(_) => None,
            };
            if token.is_cancelled() || !self.is_current(session) {
                return;
            }
            if self.apply_line_up(channel, live, follow, game_id, fetched, lined) {
                return;
            }
            {
                // Decided under the track's lock, which a move takes to ask for a line-up: a
                // move either restarts the tries here or finds no line-up running.
                let mut track = lock(&follow.track);
                if track.feed.len() == seen {
                    left = left.saturating_sub(1);
                    if left == 0 {
                        track.lining_up = false;
                        return;
                    }
                } else {
                    left = self.timing.attempts;
                }
            }
            tokio::select! {
                () = tokio::time::sleep(self.timing.delay) => {}
                () = follow.moved.notified() => {}
                () = token.cancelled() => {}
            }
        }
    }

    /// The export's moves lined up with what the feed has shown so far, and how many positions
    /// the feed had shown then. Worked out away from the track's lock: finding the moves the
    /// export leaves out searches thousands of positions.
    async fn line_up_export(&self, game: &TvGame, follow: &TvFollow) -> Option<Lined> {
        let setup = game.setup.clone()?;
        let san = game.san.clone();
        let (feed, raw, live) = {
            let track = lock(&follow.track);
            (track.feed.clone(), track.raw.clone(), track.live.clone())
        };
        let seen = feed.len();
        let moves = tokio::task::spawn_blocking(move || tv_moves(&setup, &san, &feed, &raw, &live))
            .await
            .unwrap_or(TvMoves::Unknown);
        Some(Lined { seen, moves })
    }

    /// Adopts an export: the time control and rated flag, and the moves `lined` found. A failure
    /// shows where the moves go. True when there is nothing more to wait for: every move, a
    /// variant KChess does not play, or another game.
    fn apply_line_up(
        &self,
        channel: &str,
        live: &Arc<Mutex<TvLive>>,
        follow: &TvFollow,
        game_id: &str,
        fetched: CoreResult<TvGame>,
        lined: Option<Lined>,
    ) -> bool {
        let mut state = lock(live);
        let Some(current) = state.current.as_mut() else {
            return true;
        };
        if current.game_id != game_id {
            return true;
        }
        let before = current.clone();
        let mut track = lock(&follow.track);
        let done = match fetched {
            Err(cause) => {
                current.moves_error = Some(said_or(cause, "Lichess did not send the moves."));
                false
            }
            Ok(game) => {
                current.moves_error = None;
                current.rated = game.details.rated;
                current.speed = game.details.speed.clone();
                current.clock = game.details.clock;
                match &game.setup {
                    None => {
                        track.unplayable = true;
                        current.moves_error = Some(UNPLAYABLE.to_string());
                        true
                    }
                    Some(setup) => {
                        current.variant = setup.variant.key().to_string();
                        // Brought up to the feed as it is now, under the lock its moves take.
                        let moves = lined.map_or(TvMoves::Unknown, |lined| lined.caught_up(&track));
                        let (moves, behind) = match moves {
                            TvMoves::Lined(moves) => (Some(moves), None),
                            TvMoves::Behind(moves, behind) => (Some(moves), Some(behind)),
                            TvMoves::Unknown => (None, None),
                        };
                        let lined_up = moves.is_some() && behind.is_none();
                        // An export that says less than the list already shows changes nothing.
                        let known = |moves: &Option<Vec<String>>, behind: Option<u32>| {
                            moves.as_ref().map(|moves| (moves.len(), behind.is_none()))
                        };
                        if moves.is_some()
                            && known(&moves, behind) > known(&current.moves, current.moves_behind)
                        {
                            current.start_fen = Some(setup.fen.clone());
                            current.moves = moves;
                            current.moves_behind = behind;
                        }
                        lined_up
                    }
                }
            }
        };
        if done {
            track.lining_up = false;
        }
        drop(track);
        if *current != before {
            let frame = current.clone();
            self.emit_tv(channel, frame);
        }
        done
    }

    async fn fetch_tv_game(&self, id: &str, token: &CancellationToken) -> CoreResult<TvGame> {
        let query = [
            ("moves", "true".to_string()),
            ("tags", "false".to_string()),
            ("clocks", "false".to_string()),
            ("evals", "false".to_string()),
            ("opening", "false".to_string()),
        ];
        // Lichess exports games as PGN unless the request asks for NDJSON: one line per game.
        // One game does not wait for the bulk export lane.
        let text = self
            .lichess
            .client()
            .post("/api/games/export/_ids", &self.watcher())
            .query(&query)
            .accept("application/x-ndjson")
            .text(id)
            .cancel(token)
            .read_text(None)
            .await?;
        let line = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .ok_or_else(|| CoreError::new("Lichess has no such game."))?;
        let raw: Value = serde_json::from_str(line).map_err(|cause| {
            CoreError::new(format!("Lichess sent an unreadable game record: {cause}"))
        })?;
        Ok(parse_tv_game(&raw))
    }

    /// Keeps a game the channel moved on from, and fetches how it ended.
    fn remember(self: &Arc<Self>, channel: &str, frame: &WatchFrame, token: &CancellationToken) {
        let entry = TvPastGame {
            game_id: frame.game_id.clone(),
            white: frame.white.clone(),
            black: frame.black.clone(),
            fen: frame.fen.clone(),
            last_move: frame.last_move.clone(),
            orientation: frame.orientation,
            status: None,
            winner: None,
        };
        {
            let mut control = lock(&self.control);
            let mut list = vec![entry.clone()];
            list.extend(
                control
                    .past
                    .get(channel)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|game| game.game_id != entry.game_id)
                    .cloned(),
            );
            list.truncate(PAST_GAMES);
            control.past.insert(channel.to_string(), list);
        }
        let inner = Arc::clone(self);
        let channel = channel.to_string();
        let token = token.clone();
        tokio::spawn(async move {
            let result = inner.fetch_tv_game(&entry.game_id, &token).await;
            match result {
                Ok(game) => {
                    let mut control = lock(&inner.control);
                    if let Some(list) = control.past.get_mut(&channel)
                        && let Some(stored) = list
                            .iter_mut()
                            .find(|stored| stored.game_id == entry.game_id)
                    {
                        stored.status = game.status;
                        stored.winner = game.winner;
                    }
                }
                Err(cause) => inner.lichess.host().log(
                    Level::Debug,
                    "spectate",
                    &format!(
                        "Past game fetch failed: {} {}",
                        entry.game_id, cause.message
                    ),
                ),
            }
        });
    }

    /// Follows one game until its stream closes; answers whether the game was over by then.
    async fn follow_game(
        &self,
        id: &str,
        session: u64,
        token: &CancellationToken,
    ) -> CoreResult<bool> {
        let path = format!("/api/stream/game/{}", path_segment(id));
        let mut current: Option<WatchFrame> = None;
        self.lichess
            .client()
            .get(&path, &self.watcher())
            .lane(Lane::Live)
            .cancel(token)
            .lines(
                || {},
                |line| {
                    // Its token is cancelled as its session stops being current (`Control`).
                    if token.is_cancelled() {
                        return Ok(());
                    }
                    // A malformed record ends the game as it arrives.
                    let data: Value = serde_json::from_str(line).map_err(|cause| {
                        CoreError::new(format!("Lichess sent an unreadable game record: {cause}"))
                    })?;
                    let record: GameRecord = read(&data).unwrap_or_default();
                    let fen = display_fen(record.fen.0.as_deref().unwrap_or(""));
                    if let Some(id) = record.id.0.as_deref() {
                        let game_id = kchess_domain::js::slice_utf16(id, 12).to_string();
                        // The description has no `fen` of its own: the game is shown from its
                        // start until the position that follows it, and the description sent
                        // again as it ends keeps the position reached.
                        let reached = current
                            .as_ref()
                            .filter(|frame| fen.is_none() && frame.game_id == game_id);
                        let last_move = uci_or_none(record.last_move.as_ref())
                            .or_else(|| reached.and_then(|frame| frame.last_move.clone()));
                        let Some(fen) = fen
                            .or_else(|| reached.map(|frame| frame.fen.clone()))
                            .or_else(|| display_fen(record.initial_fen.0.as_deref()?))
                            .or_else(|| display_fen(STANDARD_START))
                        else {
                            return Ok(());
                        };
                        // The description of the game: first, and again when it ends.
                        let status = record.status.0.and_then(|status| status.name.0);
                        let players = record.players.0.unwrap_or_default();
                        let winner = record
                            .winner
                            .0
                            .as_deref()
                            .and_then(kchess_domain::color::from_name);
                        let finished = status
                            .as_deref()
                            .is_some_and(|status| status != "created" && status != "started");
                        let details = parse_game_details(&data);
                        current = Some(WatchFrame {
                            session,
                            source: WatchSource::Game,
                            channel: None,
                            game_id,
                            white: player_of(players.get("white")),
                            black: player_of(players.get("black")),
                            orientation: current.as_ref().map_or(Color::White, |c| c.orientation),
                            fen,
                            last_move,
                            white_clock: current.as_ref().and_then(|c| c.white_clock),
                            black_clock: current.as_ref().and_then(|c| c.black_clock),
                            variant: {
                                let key = record.variant.0.and_then(|variant| variant.key.0);
                                kchess_domain::variant::Variant::from_lichess(key.as_deref())
                                    .map_or_else(|| key.unwrap_or_default(), |v| v.key().into())
                            },
                            speed: details.speed,
                            rated: details.rated,
                            clock: details.clock,
                            status,
                            winner,
                            finished,
                            start_fen: None,
                            moves: None,
                            moves_behind: None,
                            moves_error: None,
                            previous: None,
                        });
                    } else if let (Some(fen), Some(previous)) = (fen, current.as_ref()) {
                        current = Some(WatchFrame {
                            fen,
                            last_move: uci_or_none(record.lm.as_ref()),
                            white_clock: record.wc.0.or(previous.white_clock),
                            black_clock: record.bc.0.or(previous.black_clock),
                            ..previous.clone()
                        });
                    }
                    if let Some(frame) = current.clone() {
                        self.sink.state(WatchState {
                            session,
                            phase: WatchPhase::Connected,
                            message: None,
                            kind: None,
                        });
                        self.sink.frame(frame);
                    }
                    Ok(())
                },
            )
            .await?;
        Ok(current.is_some_and(|frame| frame.finished))
    }

    /// `watchRound`'s feed: the PGN text is read as it arrives and split at the
    /// two blank lines Lichess ends every game with. Each read hands over the games that parsed,
    /// so the sink gets one batch per chunk; the last part of the feed is taken at its end.
    async fn follow_round(
        &self,
        round_id: &str,
        session: u64,
        token: &CancellationToken,
    ) -> CoreResult<()> {
        let path = format!("/api/stream/broadcast/round/{}.pgn", path_segment(round_id));
        let mut buffer = String::new();
        let deliver = |games: Vec<BroadcastGame>| {
            if !games.is_empty() && self.is_current(session) && !token.is_cancelled() {
                self.sink.broadcast(BroadcastUpdate {
                    session,
                    round_id: round_id.to_string(),
                    games,
                    ended: None,
                    error: None,
                });
            }
        };
        self.lichess
            .client()
            .get(&path, &self.watcher())
            .lane(Lane::Live)
            .cancel(token)
            .chunks(|chunk| {
                if token.is_cancelled() || !self.is_current(session) {
                    return Ok(());
                }
                buffer.push_str(chunk);
                if buffer.len() > MAX_PGN {
                    let cut = buffer.len() - MAX_PGN;
                    let cut = (cut..buffer.len())
                        .find(|i| buffer.is_char_boundary(*i))
                        .unwrap_or(buffer.len());
                    buffer.drain(..cut);
                }
                deliver(take_pgn_games(&mut buffer, false));
                Ok(())
            })
            .await?;
        deliver(take_pgn_games(&mut buffer, true));
        Ok(())
    }
}

/// The games of a PGN buffer: every part before the last two-blank-line
/// separator parses into a game; the last part stays in the buffer until more text arrives, unless
/// the feed has ended, when it is parsed too.
fn take_pgn_games(buffer: &mut String, finished: bool) -> Vec<BroadcastGame> {
    let text = std::mem::take(buffer);
    let mut parts: Vec<&str> = text.split("\n\n\n").collect();
    let rest = if finished { None } else { parts.pop() };
    let games = parts.into_iter().filter_map(broadcast_game).collect();
    if let Some(rest) = rest {
        buffer.push_str(rest);
    }
    games
}
