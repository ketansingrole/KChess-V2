//! Watch → TV over local fixtures: how soon a featured game's moves show, however Lichess's
//! delayed export answers. Lichess leaves an ongoing game's last three plies out of its export
//! (measured on every channel), so the moves between the export and the feed are worked out from
//! the feed's position, and the board never waits for them.

use crate::fixtures;
use crate::support::wait_until;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{MemoryStore, MemoryTokens, Recording, Reply, Request, lichess_service, serve};
use kchess_core::lichess::watch::{
    BroadcastUpdate, Spectator, WatchFrame, WatchSink, WatchState, WatchTarget,
};
use kchess_domain::replay;
use kchess_domain::variant::Variant;
use serde_json::json;
use tokio::sync::{mpsc, watch};

const INITIAL: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// How long moves may take to show: far less than the five seconds before a timed attempt, so
/// the first export is the one that shows them.
const PROMPT: Duration = Duration::from_secs(4);

/// The game every channel shows.
const LINE: [(&str, &str); 9] = [
    ("e2e4", "e4"),
    ("e7e5", "e5"),
    ("g1f3", "Nf3"),
    ("b8c6", "Nc6"),
    ("f1c4", "Bc4"),
    ("f8c5", "Bc5"),
    ("e1g1", "O-O"),
    ("g8f6", "Nf6"),
    ("c2c3", "c3"),
];

fn uci(plies: usize) -> Vec<String> {
    LINE[..plies]
        .iter()
        .map(|(uci, _)| uci.to_string())
        .collect()
}

/// The position after `ply` moves, as the feed sends it: with its clock and move number.
fn fen_after(ply: usize) -> String {
    replay::replay_setup(Variant::Standard, INITIAL, &uci(ply))
        .unwrap()
        .fen
}

/// The board and side to move: the frames' FEN keeps only these.
fn placement(fen: &str) -> String {
    fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
}

fn featured(id: &str, ply: usize) -> String {
    let record = json!({
        "t": "featured",
        "d": {
            "id": id,
            "fen": fen_after(ply),
            "orientation": "white",
            "players": [
                { "color": "white", "seconds": 600, "user": { "name": "Alice" }, "rating": 2300 },
                { "color": "black", "seconds": 600, "user": { "name": "Bob" }, "rating": 2250 }
            ]
        }
    });
    format!("{record}\n")
}

fn moved(ply: usize) -> String {
    let (lm, _) = LINE[ply - 1];
    let record =
        json!({ "t": "fen", "d": { "fen": fen_after(ply), "lm": lm, "wc": 500, "bc": 500 } });
    format!("{record}\n")
}

/// Lichess's export of the game three plies short of `ply`, as it answers an ongoing game.
fn export_behind(ply: usize) -> Reply {
    let san: Vec<&str> = LINE[..ply.saturating_sub(3)]
        .iter()
        .map(|(_, san)| *san)
        .collect();
    let game = json!({
        "variant": "standard", "speed": "blitz", "rated": true, "status": "started",
        "clock": { "initial": 180, "increment": 0 },
        "moves": san.join(" "),
    });
    Reply::Json(200, game.to_string())
}

#[derive(Default)]
struct Sink {
    frames: Mutex<Vec<WatchFrame>>,
}

impl WatchSink for Sink {
    fn frame(&self, frame: WatchFrame) {
        self.frames.lock().unwrap().push(frame);
    }

    fn state(&self, _: WatchState) {}

    fn broadcast(&self, _: BroadcastUpdate) {}
}

impl Sink {
    /// The latest frame of `session` once `holds`, waiting at most `limit`.
    async fn frame(
        &self,
        limit: Duration,
        what: &str,
        session: u64,
        holds: impl Fn(&WatchFrame) -> bool,
    ) -> WatchFrame {
        wait_until(limit, what, || {
            let frames = self.frames.lock().unwrap();
            frames
                .iter()
                .rev()
                .find(|frame| frame.session == session)
                .filter(|frame| holds(frame))
                .cloned()
        })
        .await
    }
}

/// TV channels whose feeds the test writes to, the plies each channel's game has reached, and
/// how many exports Lichess answered.
struct Tv {
    feeds: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
    plies: Arc<Mutex<HashMap<String, usize>>>,
    exports: Arc<AtomicUsize>,
    sink: Arc<Sink>,
    spectator: Spectator,
    _fixture: fixtures::Fixture,
}

impl Tv {
    /// A Lichess whose channel `c` features game `c` (its id padded to 8 characters), and whose
    /// export `answer` replies to, given the game and the ply its feed has reached.
    async fn new(answer: impl Fn(&str, usize) -> Reply + Send + 'static) -> Tv {
        let feeds: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>> = Arc::default();
        let plies: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let exports = Arc::new(AtomicUsize::new(0));
        let fixture = serve({
            let (feeds, plies, exports) =
                (Arc::clone(&feeds), Arc::clone(&plies), Arc::clone(&exports));
            move |seen: &Request| {
                if let Some(channel) = seen
                    .path()
                    .strip_prefix("/api/tv/")
                    .and_then(|rest| rest.strip_suffix("/feed"))
                {
                    let (tx, rx) = mpsc::unbounded_channel();
                    feeds.lock().unwrap().insert(channel.to_string(), tx);
                    return Reply::Stream(rx);
                }
                if seen.path() == "/api/games/export/_ids" {
                    exports.fetch_add(1, Ordering::SeqCst);
                    let game = seen.body.trim().to_string();
                    let ply = plies
                        .lock()
                        .unwrap()
                        .get(game.trim_end_matches('-'))
                        .copied()
                        .unwrap_or(0);
                    return answer(&game, ply);
                }
                Reply::Hang
            }
        })
        .await;
        let host = Arc::new(Recording::default());
        let sink = Arc::new(Sink::default());
        let spectator = Spectator::new(
            lichess_service(
                &fixture.base,
                &host,
                MemoryStore::with_accounts(&[]),
                MemoryTokens::with(&[]),
            ),
            Arc::clone(&sink) as Arc<dyn WatchSink>,
        );
        Tv {
            feeds,
            plies,
            exports,
            sink,
            spectator,
            _fixture: fixture,
        }
    }

    /// The game a channel features: its name padded to Lichess's eight characters.
    fn game(channel: &str) -> String {
        format!("{channel:-<8}")
    }

    /// Starts watching `channel`, whose feed begins with its game at `ply`.
    async fn watch(&self, channel: &str, ply: usize) -> u64 {
        self.plies.lock().unwrap().insert(channel.to_string(), ply);
        let session = self
            .spectator
            .watch(WatchTarget::Channel(channel.to_string()));
        let feed = wait_until(PROMPT, "the feed", || {
            self.feeds.lock().unwrap().get(channel).cloned()
        })
        .await;
        feed.send(featured(&Tv::game(channel), ply)).unwrap();
        session
    }

    /// The channel's game reaches `ply`.
    fn play(&self, channel: &str, ply: usize) {
        self.plies.lock().unwrap().insert(channel.to_string(), ply);
        let feed = self.feeds.lock().unwrap().get(channel).cloned().unwrap();
        // A channel no longer watched has hung up: its move goes nowhere.
        feed.send(moved(ply)).ok();
    }
}

fn lined(frame: &WatchFrame, plies: usize) -> bool {
    frame.moves.as_ref() == Some(&uci(plies)) && frame.moves_behind.is_none()
}

/// The three plies the export leaves out are Nc6, Bc4 and Bc5: Black's two moves could have come
/// in either order. The moves the export has show at once, short of the board, and the next
/// move's export names the rest.
#[tokio::test]
async fn moves_that_could_have_come_in_either_order_show_up_to_them_until_the_next_move() {
    let tv = Tv::new(|_, ply| export_behind(ply)).await;
    let session = tv.watch("blitz", 6).await;
    let shown = tv
        .sink
        .frame(PROMPT, "the exported moves", session, |frame| {
            frame.moves.is_some()
        })
        .await;
    assert_eq!(shown.moves, Some(uci(3)));
    assert_eq!(shown.moves_behind, Some(3));
    assert_eq!(placement(&shown.fen), placement(&fen_after(6)));
    assert_eq!(shown.speed.as_deref(), Some("blitz"));
    tv.play("blitz", 7);
    let caught_up = tv
        .sink
        .frame(PROMPT, "every move", session, |frame| lined(frame, 7))
        .await;
    assert_eq!(caught_up.last_move.as_deref(), Some("e1g1"));
    // Once every move is known, the next ones join the list without another export.
    let exports = tv.exports.load(Ordering::SeqCst);
    tv.play("blitz", 8);
    tv.sink
        .frame(PROMPT, "the next move", session, |frame| lined(frame, 8))
        .await;
    assert_eq!(tv.exports.load(Ordering::SeqCst), exports);
    tv.spectator.stop();
}

/// An export that takes its time holds up only the move list: the board follows the feed, and
/// the moves played while the export was on its way join the list when it comes.
#[tokio::test]
async fn a_slow_export_leaves_the_board_live_and_its_moves_catch_up_when_it_answers() {
    let (open, gate) = watch::channel(false);
    let tv = Tv::new(move |_, ply| Reply::Gated(gate.clone(), Box::new(export_behind(ply)))).await;
    let session = tv.watch("rapid", 3).await;
    let first = tv.sink.frame(PROMPT, "the board", session, |_| true).await;
    assert_eq!(placement(&first.fen), placement(&fen_after(3)));
    assert_eq!(first.moves, None);
    for ply in 4..=5 {
        tv.play("rapid", ply);
        tv.sink
            .frame(PROMPT, "the move on the board", session, |frame| {
                placement(&frame.fen) == placement(&fen_after(ply))
            })
            .await;
    }
    open.send(true).unwrap();
    // The export answered for ply 3 shows Lichess's moves up to ply 0: the bridge reaches the
    // feed's first position, and the two moves since follow it.
    tv.sink
        .frame(PROMPT, "the moves", session, |frame| lined(frame, 5))
        .await;
    tv.spectator.stop();
}

/// A rate-limited export says why the moves are missing, never "Loading moves…" for good, and
/// the moves come with the next export Lichess answers.
#[tokio::test]
async fn a_rate_limited_export_says_why_and_the_moves_come_once_lichess_answers() {
    // Only the first export is refused.
    let limited = Arc::new(AtomicBool::new(true));
    let tv = Tv::new({
        let limited = Arc::clone(&limited);
        move |_, ply| {
            if limited.swap(false, Ordering::SeqCst) {
                Reply::WithHeaders(
                    429,
                    vec![("Retry-After", "1".into())],
                    json!({ "error": "Too many requests" }).to_string(),
                )
            } else {
                export_behind(ply)
            }
        }
    })
    .await;
    let session = tv.watch("classical", 4).await;
    let refused = tv
        .sink
        .frame(PROMPT, "why the moves are missing", session, |frame| {
            frame.moves_error.is_some()
        })
        .await;
    assert_eq!(refused.moves, None);
    assert!(
        refused
            .moves_error
            .as_deref()
            .is_some_and(|error| error.contains("slow down")),
        "{refused:?}"
    );
    // The players think; the next move asks again once the cooldown is over.
    tv.play("classical", 5);
    let shown = tv
        .sink
        .frame(PROMPT, "the moves", session, |frame| lined(frame, 5))
        .await;
    assert_eq!(shown.moves_error, None);
    tv.spectator.stop();
}

/// Switching channels quickly: each channel's moves show for its own game, and the channel left
/// behind reports nothing more.
#[tokio::test]
async fn switching_channels_quickly_shows_each_channels_own_moves() {
    let tv = Tv::new(|_, ply| export_behind(ply)).await;
    let mut sessions = Vec::new();
    for (channel, ply) in [("bullet", 4), ("blitz", 5), ("rapid", 3)] {
        sessions.push((tv.watch(channel, ply).await, channel, ply));
    }
    let &(session, channel, ply) = sessions.last().unwrap();
    let shown = tv
        .sink
        .frame(PROMPT, "the last channel's moves", session, |frame| {
            frame.moves.is_some()
        })
        .await;
    assert_eq!(shown.game_id, Tv::game(channel));
    assert!(lined(&shown, ply), "{shown:?}");
    let count = tv.sink.frames.lock().unwrap().len();
    // The channels left behind send moves no one is watching.
    tv.play("bullet", 5);
    tv.play("blitz", 6);
    tv.play(channel, ply + 1);
    tv.sink
        .frame(PROMPT, "the watched channel's move", session, |frame| {
            lined(frame, ply + 1)
        })
        .await;
    let frames = tv.sink.frames.lock().unwrap()[count..].to_vec();
    assert!(
        frames.iter().all(|frame| frame.session == session),
        "a channel left behind reported: {frames:?}"
    );
    tv.spectator.stop();
}

/// After a long think the feed reconnects with the game moved on: the moves it missed are not
/// named, so the list keeps what it had, short of the board, until an export names them.
#[tokio::test]
async fn a_feed_that_missed_moves_keeps_its_list_until_the_export_names_them() {
    let tv = Tv::new(|_, ply| export_behind(ply)).await;
    let session = tv.watch("blitz", 3).await;
    tv.sink
        .frame(PROMPT, "the moves", session, |frame| lined(frame, 3))
        .await;
    // The feed comes back two plies on: Nc6 and Bc4 went unseen, and Nf3 and Bc4 could have
    // come in either order, so only the next moves' exports name them.
    let feed = tv.feeds.lock().unwrap().get("blitz").cloned().unwrap();
    tv.plies.lock().unwrap().insert("blitz".into(), 5);
    feed.send(featured(&Tv::game("blitz"), 5)).unwrap();
    let short = tv
        .sink
        .frame(PROMPT, "the list short of the board", session, |frame| {
            placement(&frame.fen) == placement(&fen_after(5))
        })
        .await;
    assert_eq!(short.moves, Some(uci(3)));
    assert_eq!(short.moves_behind, Some(2));
    tv.play("blitz", 6);
    tv.play("blitz", 7);
    tv.sink
        .frame(PROMPT, "every move", session, |frame| lined(frame, 7))
        .await;
    let frames = tv.sink.frames.lock().unwrap().clone();
    let after: Vec<_> = frames.iter().skip_while(|frame| !lined(frame, 3)).collect();
    assert!(
        after.iter().all(|frame| frame.moves.is_some()),
        "the list went back to loading: {after:?}"
    );
    tv.spectator.stop();
}
