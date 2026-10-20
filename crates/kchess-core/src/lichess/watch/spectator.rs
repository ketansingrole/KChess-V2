//! The spectator: one watch at a time, its streams and what it reports (moved from `watch.rs`
//! without changes).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::feed::{player_of, position_key, take_utf16, uci_or_none};
use super::{
    BroadcastGame, BroadcastUpdate, LineUpTiming, MAX_PGN, PAST_GAMES, TvGame, TvPastGame,
    WatchFrame, WatchSink, WatchState, WatchTarget, align_tv_moves, broadcast_game, display_fen,
    parse_game_details, parse_tv_game,
};
use crate::error::{CoreError, Result as CoreResult};
use crate::host::Level;
use crate::lichess::accounts::{Lichess, path_segment};
use crate::lichess::client::{Failure, parse_json};
use crate::lichess::decode::{Loose, read};
use crate::lichess::stream::STALLED;

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

/// What a viewer is told when a feed's task panicked (the panic hook logged it).
const FEED_PANICKED: &str = "The feed stopped after an internal error. Retry to reconnect.";

/* ── Spectator ── */

#[derive(Default)]
struct Control {
    session: u64,
    cancel: Option<CancellationToken>,
    /// Earlier games per TV channel, kept for the app's lifetime.
    past: HashMap<String, Vec<TvPastGame>>,
}

/// The TV game being followed, with what the feed has shown of it.
#[derive(Default)]
struct TvLive {
    current: Option<WatchFrame>,
    track: Option<Arc<Mutex<TvTrack>>>,
}

#[derive(Default)]
struct TvTrack {
    feed: Vec<String>,
    live: Vec<Option<String>>,
}

struct Inner {
    lichess: Arc<Lichess>,
    sink: Arc<dyn WatchSink>,
    timing: LineUpTiming,
    control: Mutex<Control>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

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

    /// Watches a TV channel or a game; returns the session number its frames carry.
    pub fn watch(&self, target: WatchTarget) -> u64 {
        let (session, token) = self.inner.begin();
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            inner.report(session, &token, "connecting", None);
            let lichess = Arc::clone(&inner.lichess);
            let followed = lichess.client().policy().with_usage("", "watch", async {
                match &target {
                    WatchTarget::Channel(channel) => {
                        inner.follow_tv(channel, session, &token).await
                    }
                    WatchTarget::Game(id) => inner.follow_game(id, session, &token).await,
                }
            });
            // A panic ends the feed as a failure, so the viewer is offered a retry.
            let result = crate::tasks::caught(followed)
                .await
                .unwrap_or_else(|_| Err(CoreError::new(FEED_PANICKED)));
            match result {
                Ok(()) => inner.report(
                    session,
                    &token,
                    "ended",
                    Some("The feed ended. Retry to reconnect."),
                ),
                Err(cause) => {
                    let label = match &target {
                        WatchTarget::Channel(channel) => channel.clone(),
                        WatchTarget::Game(id) => id.clone(),
                    };
                    inner.host_log(
                        Level::Warn,
                        &format!("Watch failed: {label} {}", cause.message),
                    );
                    let message = if cause.message.is_empty() {
                        "The feed disconnected. Retry to reconnect.".to_string()
                    } else {
                        cause.message
                    };
                    inner.report(session, &token, "error", Some(&message));
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
            let lichess = Arc::clone(&inner.lichess);
            let followed = lichess.client().policy().with_usage(
                "",
                "watch",
                inner.follow_round(&round_id, session, &token),
            );
            let result = crate::tasks::caught(followed)
                .await
                .unwrap_or_else(|_| Err(CoreError::new(FEED_PANICKED)));
            match result {
                Ok(()) => {
                    if inner.is_current(session) {
                        inner.sink.broadcast(BroadcastUpdate {
                            session,
                            round_id: round_id.clone(),
                            games: Vec::new(),
                            ended: Some(true),
                            error: None,
                        });
                    }
                }
                Err(cause) => {
                    inner.host_log(
                        Level::Warn,
                        &format!("Broadcast feed failed: {round_id} {}", cause.message),
                    );
                    if inner.is_current(session) && !token.is_cancelled() {
                        let message = if cause.message.is_empty() {
                            "The broadcast feed stopped.".to_string()
                        } else {
                            cause.message
                        };
                        inner.sink.broadcast(BroadcastUpdate {
                            session,
                            round_id: round_id.clone(),
                            games: Vec::new(),
                            ended: Some(true),
                            error: Some(message),
                        });
                    }
                }
            }
        });
        session
    }
}

impl Inner {
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
        phase: &'static str,
        message: Option<&str>,
    ) {
        if !token.is_cancelled() && self.is_current(session) {
            self.sink.state(WatchState {
                session,
                phase,
                message: message.map(str::to_string),
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
                .ndjson(
                    reqwest::Method::GET,
                    &path,
                    &[],
                    None,
                    None,
                    token,
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
                Err(Failure::Core(cause))
                    if cause.message == STALLED
                        && !token.is_cancelled()
                        && self.is_current(session) => {}
                other => return other.map_err(Into::into),
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
                let Some(id) = data.id.0.as_deref() else {
                    return Ok(());
                };
                let game_id = take_utf16(id, 12);
                let white = side("white");
                let black = side("black");
                let mut state = lock(live);
                if let Some(previous) = state.current.as_ref()
                    && previous.game_id != game_id
                {
                    let previous = previous.clone();
                    self.remember(channel, &previous, token);
                }
                let frame = WatchFrame {
                    session,
                    source: "tv",
                    channel: Some(channel.to_string()),
                    game_id: game_id.clone(),
                    white: player_of(white.as_ref()),
                    black: player_of(black.as_ref()),
                    orientation: if data.orientation.0.as_deref() == Some("black") {
                        "black"
                    } else {
                        "white"
                    },
                    fen: fen.clone(),
                    last_move: None,
                    white_clock: white
                        .as_ref()
                        .and_then(|p| p.get("seconds"))
                        .and_then(Value::as_f64),
                    black_clock: black
                        .as_ref()
                        .and_then(|p| p.get("seconds"))
                        .and_then(Value::as_f64),
                    variant: channel.to_string(),
                    speed: None,
                    rated: None,
                    clock: None,
                    status: None,
                    winner: None,
                    finished: false,
                    start_fen: None,
                    moves: None,
                    previous: None,
                };
                state.current = Some(frame.clone());
                let track = Arc::new(Mutex::new(TvTrack {
                    feed: vec![position_key(&fen)],
                    live: Vec::new(),
                }));
                state.track = Some(Arc::clone(&track));
                drop(state);
                self.sink.state(WatchState {
                    session,
                    phase: "connected",
                    message: None,
                });
                self.emit_tv(channel, frame);
                let inner = Arc::clone(self);
                let live = Arc::clone(live);
                let token = token.clone();
                let channel = channel.to_string();
                tokio::spawn(async move {
                    let lichess = Arc::clone(&inner.lichess);
                    lichess
                        .client()
                        .policy()
                        .with_usage(
                            "",
                            "watch",
                            inner.line_up(&channel, &live, &track, &game_id, session, &token),
                        )
                        .await;
                });
                Ok(())
            }
            TvRecord::Fen(data) => {
                let Some(fen) = fen_of(&data.fen) else {
                    return Ok(());
                };
                let last_move = uci_or_none(data.lm.as_ref());
                let mut state = lock(live);
                let (Some(current), Some(track)) = (state.current.clone(), state.track.clone())
                else {
                    return Ok(());
                };
                {
                    let mut track = lock(&track);
                    track.feed.push(position_key(&fen));
                    track.live.push(last_move.clone());
                }
                let frame = WatchFrame {
                    fen,
                    last_move: last_move.clone(),
                    white_clock: data.wc.0.or(current.white_clock),
                    black_clock: data.bc.0.or(current.black_clock),
                    // A move the feed did not name breaks the list; better none than a wrong one.
                    moves: match (current.moves.as_ref(), last_move.as_ref()) {
                        (Some(moves), Some(last)) => {
                            let mut moves = moves.clone();
                            moves.push(last.clone());
                            Some(moves)
                        }
                        _ => None,
                    },
                    ..current
                };
                state.current = Some(frame.clone());
                drop(state);
                self.sink.state(WatchState {
                    session,
                    phase: "connected",
                    message: None,
                });
                self.emit_tv(channel, frame);
                Ok(())
            }
            TvRecord::Other => Ok(()),
        }
    }

    /// Fills in what the TV feed leaves out: the time control, rated flag and, once Lichess's
    /// delayed export reaches a position the feed showed, the moves so far. Failures stay quiet.
    async fn line_up(
        self: &Arc<Self>,
        channel: &str,
        live: &Arc<Mutex<TvLive>>,
        track: &Arc<Mutex<TvTrack>>,
        game_id: &str,
        session: u64,
        token: &CancellationToken,
    ) {
        for attempt in 0..self.timing.attempts {
            if attempt > 0 {
                tokio::select! {
                    _ = tokio::time::sleep(self.timing.delay) => {}
                    _ = token.cancelled() => {}
                }
            }
            if token.is_cancelled() || !self.is_current(session) {
                return;
            }
            let game = match self.fetch_tv_game(game_id, token).await {
                Ok(game) => game,
                Err(cause) => {
                    self.lichess.host().log(
                        Level::Debug,
                        "spectate",
                        &format!("TV lineup fetch failed: {game_id} {}", cause.message),
                    );
                    continue;
                }
            };
            if token.is_cancelled() || !self.is_current(session) {
                return;
            }
            let snapshot = {
                let track = lock(track);
                (track.feed.clone(), track.live.clone())
            };
            let moves = game
                .setup
                .as_ref()
                .and_then(|setup| align_tv_moves(setup, &game.san, &snapshot.0, &snapshot.1));
            if attempt == 0 || moves.is_some() {
                self.apply_line_up(channel, live, game_id, &game, moves.as_deref());
            }
            if moves.is_some() || game.setup.is_none() {
                return;
            }
        }
    }

    fn apply_line_up(
        &self,
        channel: &str,
        live: &Arc<Mutex<TvLive>>,
        game_id: &str,
        game: &TvGame,
        moves: Option<&[String]>,
    ) {
        let mut state = lock(live);
        let Some(current) = state.current.as_mut() else {
            return;
        };
        if current.game_id != game_id {
            return;
        }
        current.rated = game.details.rated;
        current.speed = game.details.speed.clone();
        current.clock = game.details.clock;
        if let Some(setup) = &game.setup {
            current.variant = setup.variant.to_string();
        }
        if let (Some(setup), Some(moves)) = (&game.setup, moves) {
            current.start_fen = Some(setup.fen.clone());
            current.moves = Some(moves.to_vec());
        }
        let frame = current.clone();
        drop(state);
        self.emit_tv(channel, frame);
    }

    async fn fetch_tv_game(&self, id: &str, token: &CancellationToken) -> CoreResult<TvGame> {
        let path = "/api/games/export/_ids";
        let query = [
            ("moves", "true".to_string()),
            ("tags", "false".to_string()),
            ("clocks", "false".to_string()),
            ("evals", "false".to_string()),
            ("opening", "false".to_string()),
        ];
        // Lichess exports games as PGN unless the request asks for NDJSON: one line per game.
        let text = self
            .lichess
            .client()
            .export_one(id, &query, "application/x-ndjson", token)
            .await
            .map_err(CoreError::from)?;
        let line = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let raw: Value = parse_json(line, path).map_err(CoreError::from)?;
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
            let lichess = Arc::clone(&inner.lichess);
            let result = lichess
                .client()
                .policy()
                .with_usage("", "watch", inner.fetch_tv_game(&entry.game_id, &token))
                .await;
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

    async fn follow_game(
        &self,
        id: &str,
        session: u64,
        token: &CancellationToken,
    ) -> CoreResult<()> {
        let path = format!("/api/stream/game/{}", path_segment(id));
        let mut current: Option<WatchFrame> = None;
        self.lichess
            .client()
            .ndjson(
                reqwest::Method::GET,
                &path,
                &[],
                None,
                None,
                token,
                |line| {
                    if token.is_cancelled() || !self.is_current(session) {
                        return Ok(());
                    }
                    // A malformed record ends the game as it arrives.
                    let data: Value = serde_json::from_str(line).map_err(|cause| {
                        CoreError::new(format!("Lichess sent an unreadable game record: {cause}"))
                    })?;
                    let record: GameRecord = read(&data).unwrap_or_default();
                    let Some(fen) = display_fen(record.fen.0.as_deref().unwrap_or("")) else {
                        return Ok(());
                    };
                    if let Some(id) = record.id.0.as_deref() {
                        // The description of the game: first, and again when it ends.
                        let status = record.status.0.and_then(|status| status.name.0);
                        let players = record.players.0.unwrap_or_default();
                        let winner = match record.winner.0.as_deref() {
                            Some("white") => Some("white"),
                            Some("black") => Some("black"),
                            _ => None,
                        };
                        let finished = status
                            .as_deref()
                            .is_some_and(|status| status != "created" && status != "started");
                        let details = parse_game_details(&data);
                        current = Some(WatchFrame {
                            session,
                            source: "game",
                            channel: None,
                            game_id: take_utf16(id, 12),
                            white: player_of(players.get("white")),
                            black: player_of(players.get("black")),
                            orientation: current.as_ref().map_or("white", |c| c.orientation),
                            fen,
                            last_move: uci_or_none(record.last_move.as_ref()),
                            white_clock: current.as_ref().and_then(|c| c.white_clock),
                            black_clock: current.as_ref().and_then(|c| c.black_clock),
                            variant: record
                                .variant
                                .0
                                .and_then(|variant| variant.key.0)
                                .unwrap_or_else(|| "standard".into()),
                            speed: details.speed,
                            rated: details.rated,
                            clock: details.clock,
                            status,
                            winner,
                            finished,
                            start_fen: None,
                            moves: None,
                            previous: None,
                        });
                    } else if let Some(previous) = current.as_ref() {
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
                            phase: "connected",
                            message: None,
                        });
                        self.sink.frame(frame);
                    }
                    Ok(())
                },
            )
            .await?;
        Ok(())
    }

    /// `watchRound`'s feed (`readPgnStream`): the PGN text is read as it arrives and split at the
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
            .stream_chunks(&path, None, token, |chunk| {
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

/// The games of a PGN buffer (`flush` in `readPgnStream`): every part before the last two-blank-line
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
