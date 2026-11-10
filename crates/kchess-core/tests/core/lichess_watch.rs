//! Watching Lichess over local fixtures: the spectator lifecycle, broadcasts, game details, TV
//! alignment and validation.

use crate::fixtures;
use crate::support::until;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fixtures::{
    MemoryStore, MemoryTokens, Recording, Reply, Seen, lichess_service, lichess_service_with_idle,
    serve,
};
use kchess_core::lichess::accounts::Lichess;
use kchess_core::lichess::watch::{
    BroadcastUpdate, GameSetup, LineUpTiming, Spectator, WatchFrame, WatchSink, WatchState,
    WatchTarget, align_tv_moves, broadcast_game, broadcast_summaries, parse_game_details,
    parse_tv_game,
};
use kchess_domain::{misc, replay};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const INITIAL: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const STANDARD: &str = "standard";

#[derive(Default)]
struct Sink {
    frames: Mutex<Vec<WatchFrame>>,
    states: Mutex<Vec<WatchState>>,
    broadcasts: Mutex<Vec<BroadcastUpdate>>,
}

impl WatchSink for Sink {
    fn frame(&self, frame: WatchFrame) {
        self.frames.lock().unwrap().push(frame);
    }

    fn state(&self, state: WatchState) {
        self.states.lock().unwrap().push(state);
    }

    fn broadcast(&self, update: BroadcastUpdate) {
        self.broadcasts.lock().unwrap().push(update);
    }
}

fn service(base: &str, host: &Arc<Recording>) -> Arc<Lichess> {
    lichess_service(
        base,
        host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
    )
}

#[tokio::test]
async fn reports_an_initial_http_failure_after_the_start_call_has_returned() {
    let fixture = serve(|_| Reply::Json(503, String::new())).await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    let session = spectator.watch(WatchTarget::Channel("rapid".into()));
    until("the error state", || {
        sink.states
            .lock()
            .unwrap()
            .last()
            .is_some_and(|state| state.phase == "error")
    })
    .await;
    let last = sink.states.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.session, session);
    // The page offers a retry for an outage, and shows no status.
    assert_eq!(
        last.message.as_deref(),
        Some("Lichess isn’t working right now. Try again shortly.")
    );
    assert_eq!(last.kind, Some("unavailable"));
    spectator.stop();
}

#[tokio::test]
async fn a_cancelled_watch_reports_nothing_after_it_is_stopped() {
    // Both feeds never answer: the first watch is cancelled by the second, which is stopped.
    let fixture = serve(|_| Reply::Hang).await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    let first = spectator.watch(WatchTarget::Game("Game0001".into()));
    until("the first connection", || !fixture.requests().is_empty()).await;
    let second = spectator.watch(WatchTarget::Game("Game0002".into()));
    until("the second connection", || fixture.requests().len() >= 2).await;
    spectator.stop();
    let reported = sink.states.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sink.states.lock().unwrap().len(), reported);
    assert!(
        sink.states
            .lock()
            .unwrap()
            .iter()
            .all(|state| state.session != second || state.phase != "error")
    );
    assert_ne!(first, second);
    // Stopping is not failing: the log keeps quiet about the feeds it cut.
    assert!(!host.logged().contains("failed"), "{}", host.logged());
}

#[tokio::test]
async fn a_stopped_broadcast_round_is_not_logged_as_failed() {
    let fixture = serve(|_| Reply::Hang).await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    spectator.watch_round("Round001");
    until("the round's connection", || !fixture.requests().is_empty()).await;
    spectator.stop();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!host.logged().contains("failed"), "{}", host.logged());
}

#[tokio::test]
async fn a_game_described_without_its_position_is_shown_from_the_next_one() {
    // Lichess's description of a streamed game carries `initialFen` and no `fen`: the watch page
    // once waited on "Connecting…" for an arena game, every record after it dropped.
    let fixture = serve(|seen: &Seen| {
        if seen.path() != "/api/stream/game/VuAqDosj" {
            return Reply::Hang;
        }
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let italian = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 0 1";
        for record in [
            json!({
                "id": "VuAqDosj",
                "variant": { "key": "fromPosition", "name": "From Position" },
                "speed": "rapid", "rated": true, "source": "arena", "initialFen": italian,
                "players": {
                    "white": { "user": { "name": "Thot_x_Thor" }, "rating": 2155 },
                    "black": { "user": { "name": "stefanoschor" }, "rating": 1766 }
                }
            }),
            json!({ "fen": italian, "wc": 600, "bc": 600 }),
            json!({
                "fen": "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 1 2",
                "lm": "f8c5", "wc": 600, "bc": 598
            }),
        ] {
            tx.send(format!("{record}\n")).unwrap();
        }
        std::mem::forget(tx);
        Reply::Stream(rx)
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    let session = spectator.watch(WatchTarget::Game("VuAqDosj".into()));
    until("the position after the first move", || {
        sink.frames.lock().unwrap().len() >= 3
    })
    .await;
    let frames = sink.frames.lock().unwrap().clone();
    let first = &frames[0];
    assert_eq!(first.session, session);
    assert_eq!(first.game_id, "VuAqDosj");
    assert!(
        first
            .fen
            .starts_with("r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b")
    );
    assert_eq!(
        first.variant, STANDARD,
        "a game from a position plays standard rules"
    );
    let last = frames.last().unwrap();
    assert!(last.fen.starts_with("r1bqk1nr/pppp1ppp/2n5/2b1p3/"));
    assert_eq!(last.last_move.as_deref(), Some("f8c5"));
    assert_eq!(last.black_clock, Some(598.));
    assert_eq!(last.game_id, "VuAqDosj");
    spectator.stop();
}

/// Watches one game whose stream sends `records` and closes; answers the last state reported.
async fn last_state_of_a_closed_game_stream(records: Vec<Value>) -> WatchState {
    let fixture = serve(move |seen: &Seen| {
        if seen.path() != "/api/stream/game/Game0001" {
            return Reply::Hang;
        }
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        for record in &records {
            tx.send(format!("{record}\n")).unwrap();
        }
        Reply::Stream(rx)
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    spectator.watch(WatchTarget::Game("Game0001".into()));
    until("the end of the feed", || {
        sink.states
            .lock()
            .unwrap()
            .iter()
            .any(|state| state.phase == "ended" || state.phase == "error")
    })
    .await;
    let last = sink.states.lock().unwrap().last().cloned().unwrap();
    spectator.stop();
    last
}

fn game_description(status: &str, winner: Option<&str>) -> Value {
    let mut description = json!({
        "id": "Game0001", "variant": { "key": "standard" }, "speed": "bullet",
        "status": { "name": status },
        "players": {
            "white": { "user": { "name": "donnyrules" }, "rating": 2550 },
            "black": { "user": { "name": "Solotorneos78" }, "rating": 2361 }
        }
    });
    if let Some(winner) = winner {
        description["winner"] = json!(winner);
    }
    description
}

#[tokio::test]
async fn a_game_that_ended_closes_its_feed_without_asking_to_reconnect() {
    // Lichess closes a game's stream after its result: Watch once said "The feed ended. Retry
    // to reconnect." under a game that was simply over.
    let last = last_state_of_a_closed_game_stream(vec![
        game_description("started", None),
        json!({ "fen": INITIAL, "wc": 60, "bc": 60 }),
        game_description("outoftime", Some("black")),
    ])
    .await;
    assert_eq!(last.phase, "ended");
    assert_eq!(last.message, None);
}

#[tokio::test]
async fn a_game_feed_that_closes_mid_game_offers_to_reconnect() {
    let last = last_state_of_a_closed_game_stream(vec![
        game_description("started", None),
        json!({ "fen": INITIAL, "wc": 60, "bc": 60 }),
    ])
    .await;
    assert_eq!(last.phase, "ended");
    assert!(last.message.unwrap().contains("Retry"));
}

fn tv_frame(id: &str, fen: &str) -> String {
    let frame = json!({
        "t": "featured",
        "d": {
            "id": id,
            "fen": fen,
            "orientation": "white",
            "players": [
                { "color": "white", "seconds": 600, "user": { "name": "Alice", "title": "IM" }, "rating": 2300 },
                { "color": "black", "seconds": 590, "user": { "name": "Bob" } }
            ]
        }
    });
    format!("{frame}\n")
}

#[tokio::test]
async fn a_tv_feed_adds_its_moves_once_the_delayed_export_lines_up() {
    let after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    let after_e5 = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1";
    let (feed_tx, feed_rx) = mpsc::unbounded_channel::<String>();
    let feed_rx = Mutex::new(Some(feed_rx));
    let fixture = serve(move |seen: &Seen| {
        if seen.path() == "/api/tv/rapid/feed" {
            match feed_rx.lock().unwrap().take() {
                Some(rx) => Reply::Stream(rx),
                None => Reply::Hang,
            }
        } else if seen.path() != "/api/games/export/_ids"
            || seen.header("Accept") != Some("application/x-ndjson")
        {
            // Lichess exports a game as PGN unless NDJSON is asked for; reading that as JSON
            // failed every time, so TV moves stayed "Loading moves…". Without an account,
            // `/game/export/{id}` answers a finished game with the sign-up page.
            Reply::Json(200, "[Event \"Rated rapid game\"]\n\n1. e4 *\n".into())
        } else {
            // The delayed export has only the first move so far.
            Reply::Json(
                200,
                json!({ "variant": "standard", "moves": "e4", "status": "started" }).to_string(),
            )
        }
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let timing = LineUpTiming {
        delay: Duration::from_millis(10),
        attempts: 3,
    };
    let spectator = Spectator::with_timing(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
        timing,
    );
    spectator.watch(WatchTarget::Channel("rapid".into()));
    feed_tx.send(tv_frame("abcdefghijkl", after_e4)).unwrap();
    until("the lined-up moves", || {
        sink.frames
            .lock()
            .unwrap()
            .iter()
            .any(|frame| frame.moves.is_some())
    })
    .await;
    let lined = sink
        .frames
        .lock()
        .unwrap()
        .iter()
        .find(|frame| frame.moves.is_some())
        .cloned()
        .unwrap();
    assert_eq!(lined.source, "tv");
    assert_eq!(lined.channel.as_deref(), Some("rapid"));
    assert_eq!(lined.game_id, "abcdefghijkl");
    assert_eq!(lined.white.name, "Alice");
    assert_eq!(lined.white.title.as_deref(), Some("IM"));
    assert_eq!(lined.white.rating, Some(2300.0));
    assert_eq!(lined.white_clock, Some(600.0));
    assert_eq!(lined.moves, Some(vec!["e2e4".to_string()]));
    assert_eq!(lined.start_fen.as_deref(), Some(INITIAL));
    feed_tx
        .send(format!(
            "{}\n",
            json!({ "t": "fen", "d": { "fen": after_e5, "lm": "e7e5", "wc": 599, "bc": 580 } })
        ))
        .unwrap();
    until("the second move", || {
        sink.frames
            .lock()
            .unwrap()
            .last()
            .is_some_and(|frame| frame.fen == after_e5)
    })
    .await;
    let last = sink.frames.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        last.moves,
        Some(vec!["e2e4".to_string(), "e7e5".to_string()])
    );
    assert_eq!(last.black_clock, Some(580.0));
    // A stopped watch reports nothing more.
    spectator.stop();
    let count = sink.frames.lock().unwrap().len();
    feed_tx
        .send(format!(
            "{}\n",
            json!({ "t": "fen", "d": { "fen": INITIAL, "lm": "e2e4" } })
        ))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sink.frames.lock().unwrap().len(), count);
}

#[tokio::test]
async fn a_silent_tv_feed_reconnects_instead_of_ending_the_watch() {
    // The TV feed sends no keep-alives, so a long think once ended TV with a stall error.
    let feeds = Arc::new(Mutex::new(Vec::new()));
    let senders = Arc::clone(&feeds);
    let fixture = serve(move |seen: &Seen| {
        if seen.path() != "/api/tv/rapid/feed" {
            return Reply::Hang;
        }
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        tx.send(tv_frame("abcdefghijkl", INITIAL)).unwrap();
        senders.lock().unwrap().push(tx);
        Reply::Stream(rx)
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let lichess = lichess_service_with_idle(
        &fixture.base,
        &host,
        MemoryStore::with_accounts(&[]),
        MemoryTokens::with(&[]),
        Duration::from_millis(100),
    );
    let spectator = Spectator::new(lichess, Arc::clone(&sink) as Arc<dyn WatchSink>);
    spectator.watch(WatchTarget::Channel("rapid".into()));
    until("a reconnect", || feeds.lock().unwrap().len() >= 2).await;
    assert!(
        sink.states
            .lock()
            .unwrap()
            .iter()
            .all(|state| state.phase != "error")
    );
    spectator.stop();
}

#[tokio::test]
async fn broadcast_search_lists_recent_official_broadcasts() {
    let fixture = serve(|_| {
        Reply::Json(
            200,
            json!({
                "currentPageResults": [{
                    "tour": { "id": "search01", "name": "Historical Open" },
                    "round": { "id": "round001", "name": "Final", "finishedAt": 1700000000000i64 }
                }]
            })
            .to_string(),
        )
    })
    .await;
    let host = Arc::new(Recording::default());
    let lichess = service(&fixture.base, &host);
    let found = kchess_core::lichess::watch::broadcasts(&lichess, Some(" Historical "))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tour_name, "Historical Open");
    assert_eq!(found[0].section, "past");
    let search = &fixture.requests()[0];
    assert_eq!(search.path(), "/api/broadcast/search");
    assert_eq!(search.query("q").as_deref(), Some("Historical"));
    assert_eq!(search.query("page").as_deref(), Some("1"));
}

#[test]
fn validates_watch_targets_and_broadcast_queries() {
    let channels = json!(["rapid"]);
    let sidecar = json!([]);
    let target = |value: Value| {
        misc::call(
            "assertWatchTarget",
            &[sidecar.clone(), value, channels.clone()],
        )
    };
    assert_eq!(
        target(json!({ "channel": "rapid" })).unwrap().unwrap(),
        json!({ "channel": "rapid" })
    );
    assert_eq!(
        target(json!({ "gameId": "AbCd1234" })).unwrap().unwrap(),
        json!({ "gameId": "AbCd1234" })
    );
    assert!(target(json!({ "channel": "../x" })).unwrap().is_err());
    let query = |value: Option<Value>| {
        let mut args = vec![sidecar.clone()];
        args.extend(value);
        misc::call("assertBroadcastQuery", &args).unwrap()
    };
    assert!(query(None).is_ok());
    assert_eq!(query(Some(json!(" Open "))).unwrap(), json!("Open"));
    assert!(query(Some(json!("x".repeat(101)))).is_err());
    let session =
        |value: Value| misc::call("assertWatchSession", &[sidecar.clone(), value]).unwrap();
    assert_eq!(session(json!(3)).unwrap(), json!(3));
    assert!(session(json!(0)).is_err());
    assert!(session(json!("3")).is_err());
}

#[test]
fn broadcast_summaries_classify_scheduled_rounds_and_keep_trusted_images() {
    let entries = [
        json!({
            "tour": { "id": "tour1234", "name": "Open", "image": "https://image.lichess1.org/example.webp" },
            "round": { "id": "round123", "name": "Round 4", "startsAt": 1800000000000i64 }
        }),
        json!({
            "tour": { "id": "tour5678", "name": "Live", "image": "https://untrusted.example/image.jpg" },
            "round": { "id": "round456", "name": "Round 1", "ongoing": true }
        }),
        json!({
            "tour": { "id": "tour9012", "name": "Finished" },
            "round": { "id": "round789", "name": "Final", "finished": true }
        }),
    ];
    let found = broadcast_summaries(&entries, "past");
    assert_eq!(found[0].section, "upcoming");
    assert_eq!(found[0].starts_at, Some(1800000000000));
    assert_eq!(
        found[0].image.as_deref(),
        Some("https://image.lichess1.org/example.webp")
    );
    assert_eq!(found[1].section, "active");
    assert!(found[1].ongoing);
    assert!(found[1].image.is_none());
    assert_eq!(found[2].section, "past");
    assert!(broadcast_summaries(&[json!({ "tour": { "id": 123 } })], "active").is_empty());
}

const PGN: &str = "[Event \"Test Open\"]
[White \"Alpha\"]
[Black \"Beta\"]
[WhiteElo \"2500\"]
[WhiteTitle \"GM\"]
[Result \"*\"]
[GameURL \"https://lichess.org/broadcast/test/round-1/Nhw57KE3/2AJYiCJG\"]

1. e4 { [%clk 0:30:00] } 1... e5 { [%clk 0:29:40] } 2. Nf3 { [%eval 0.2] [%clk 0:29:10] } *";

#[test]
fn reads_a_broadcast_board_players_clocks_moves_and_its_chapter_id() {
    let game = broadcast_game(PGN).unwrap();
    assert_eq!(game.id, "2AJYiCJG");
    assert_eq!(game.white.name, "Alpha");
    assert_eq!(game.white.title.as_deref(), Some("GM"));
    assert_eq!(game.white.rating, Some(2500.0));
    assert_eq!(game.black.name, "Beta");
    assert_eq!(game.moves, ["e2e4", "e7e5", "g1f3"]);
    assert_eq!(game.white_clock, Some(1750.0));
    assert_eq!(game.black_clock, Some(1780.0));
    assert!(game.ongoing);
    assert_eq!(game.start_fen, INITIAL);
    // A text without a game has no moves (the TypeScript test accepts no game as well).
    assert!(
        broadcast_game("not a game")
            .map(|g| g.moves)
            .unwrap_or_default()
            .is_empty()
    );
}

#[test]
fn keeps_the_time_control_and_rated_flag_the_tv_feed_leaves_out() {
    let details = parse_game_details(&json!({
        "id": "abcdefgh",
        "rated": true,
        "speed": "bullet",
        "clock": { "initial": 60, "increment": 0, "totalTime": 60 }
    }));
    assert_eq!(details.rated, Some(true));
    assert_eq!(details.speed.as_deref(), Some("bullet"));
    assert_eq!(
        details.clock.map(|c| (c.initial, c.increment)),
        Some((60.0, 0.0))
    );
    let correspondence = parse_game_details(&json!({ "rated": false, "speed": "correspondence" }));
    assert_eq!(correspondence.rated, Some(false));
    assert!(correspondence.clock.is_none());
    assert_eq!(parse_game_details(&json!({ "rated": "yes" })).rated, None);
}

#[test]
fn reads_the_start_moves_and_result_from_a_game_export() {
    let game = parse_tv_game(&json!({
        "variant": "chess960",
        "initialFen": "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1",
        "moves": "e4 e5",
        "status": "mate",
        "winner": "black",
        "rated": true
    }));
    let setup = game.setup.clone().unwrap();
    assert_eq!(setup.variant, "chess960");
    assert_eq!(
        setup.fen,
        "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1"
    );
    assert_eq!(game.san, ["e4", "e5"]);
    assert_eq!(game.status.as_deref(), Some("mate"));
    assert_eq!(game.winner, Some("black"));
    assert_eq!(game.details.rated, Some(true));
    // Crazyhouse replays, pockets and all; a variant KChess cannot play gets the board without a
    // move list.
    let house = parse_tv_game(&json!({ "variant": "crazyhouse", "moves": "e4" }));
    assert_eq!(house.setup.unwrap().variant, "crazyhouse");
    assert!(
        parse_tv_game(&json!({ "variant": "bughouse", "moves": "e4" }))
            .setup
            .is_none()
    );
}

/// `e4 e5 Nf3 Nc6 Bc4 Bc5` in UCI, with the feed's view (board and side) of each position.
fn line_uci() -> Vec<&'static str> {
    vec!["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5"]
}

fn san() -> Vec<String> {
    ["e4", "e5", "Nf3", "Nc6", "Bc4", "Bc5"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn key(ply: usize) -> String {
    let uci: Vec<String> = line_uci()[..ply].iter().map(|m| m.to_string()).collect();
    let fen = replay::replay_setup(STANDARD, INITIAL, &uci).unwrap().fen;
    fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
}

fn standard_setup() -> GameSetup {
    GameSetup {
        variant: "standard",
        fen: INITIAL.to_string(),
    }
}

#[test]
fn joins_a_lagging_export_to_the_moves_the_feed_sent_after_it() {
    // The feed joined after Nf3 and has since seen Nc6, Bc4 and Bc5; the export stops at Nc6.
    let feed = vec![key(3), key(4), key(5), key(6)];
    let live = vec![
        Some("b8c6".to_string()),
        Some("f1c4".to_string()),
        Some("f8c5".to_string()),
    ];
    let moves = align_tv_moves(&standard_setup(), &san()[..4], &feed, &live).unwrap();
    assert_eq!(moves, line_uci());
}

#[test]
fn waits_while_the_export_has_not_reached_anything_the_feed_showed() {
    let feed = vec![key(5), key(6)];
    assert!(
        align_tv_moves(
            &standard_setup(),
            &san()[..3],
            &feed,
            &[Some("f8c5".to_string())]
        )
        .is_none()
    );
}

#[test]
fn refuses_a_list_with_a_gap_or_that_ends_somewhere_else_than_the_board() {
    let feed = vec![key(3), key(4), key(5)];
    assert!(
        align_tv_moves(
            &standard_setup(),
            &san()[..3],
            &feed,
            &[Some("b8c6".to_string()), None]
        )
        .is_none()
    );
    assert!(
        align_tv_moves(
            &standard_setup(),
            &san()[..3],
            &feed,
            &[Some("b8c6".to_string()), Some("f1b5".to_string())]
        )
        .is_none()
    );
}

/// A round's PGN is split at the two blank lines that end each game, and each read sends the games
/// that have arrived (`readPgnStream` in the TypeScript spectator): a game that is still arriving
/// waits for its separator.
#[tokio::test]
async fn a_round_feed_is_sent_per_chunk_with_the_games_that_have_arrived() {
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut chunks = Some(chunks_rx);
    let fixture = serve(move |_| match chunks.take() {
        Some(open) => Reply::Stream(open),
        None => Reply::Hang,
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    spectator.watch_round("Round001");
    // The first game is complete; the second has only its first headers.
    chunks_tx
        .send(format!(
            "{PGN}\n\n\n[Event \"Test Open\"]\n[White \"Gamma\"]\n"
        ))
        .expect("the feed is open");
    until("the first game", || {
        sink.broadcasts.lock().unwrap().len() == 1
    })
    .await;
    // The rest of the second game arrives with its separator: one more update.
    chunks_tx
        .send("[Black \"Delta\"]\n\n1. d4 d5 *\n\n\n".to_string())
        .expect("the feed is open");
    until("the second game", || {
        sink.broadcasts.lock().unwrap().len() == 2
    })
    .await;
    let updates = sink.broadcasts.lock().unwrap().clone();
    assert_eq!(updates[0].games.len(), 1);
    assert_eq!(updates[0].games[0].white.name, "Alpha");
    assert_eq!(updates[1].games.len(), 1);
    assert_eq!(updates[1].games[0].white.name, "Gamma");
    assert_eq!(updates[1].games[0].black.name, "Delta");
    spectator.stop();
}

/// A page stops the round it was showing and opens another; the core handles the stop only after
/// the new round began (CI, 2026-11-07: an empty broadcast grid). The stop names its session, so
/// the new round keeps sending its games; stopping that one by its own session ends it.
#[tokio::test]
async fn a_stop_for_an_earlier_watch_leaves_the_newer_one_running() {
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut chunks = Some(chunks_rx);
    let fixture = serve(move |request| {
        if request.path().contains("Round002")
            && let Some(open) = chunks.take()
        {
            return Reply::Stream(open);
        }
        Reply::Hang
    })
    .await;
    let host = Arc::new(Recording::default());
    let sink = Arc::new(Sink::default());
    let spectator = Spectator::new(
        service(&fixture.base, &host),
        Arc::clone(&sink) as Arc<dyn WatchSink>,
    );
    let first = spectator.watch_round("Round001");
    until("the first round's connection", || {
        !fixture.requests().is_empty()
    })
    .await;
    let second = spectator.watch_round("Round002");
    until("both rounds' connections", || fixture.requests().len() >= 2).await;
    spectator.stop_session(first);
    chunks_tx
        .send(format!("{PGN}\n\n\n"))
        .expect("the feed is open");
    until("the newer round's game", || {
        sink.broadcasts
            .lock()
            .unwrap()
            .iter()
            .any(|update| update.session == second && update.games.len() == 1)
    })
    .await;
    spectator.stop_session(second);
    let sent = sink.broadcasts.lock().unwrap().len();
    let _ = chunks_tx.send(format!("{PGN}\n\n\n"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(sink.broadcasts.lock().unwrap().len(), sent);
}

#[test]
fn every_listed_tv_channel_is_a_valid_watch_target() {
    // Atomic, Horde and Racing Kings were listed but refused by the watch validator.
    let listed: Vec<&str> = kchess_core::lichess::watch::TV_CHANNELS
        .iter()
        .map(|(key, _)| *key)
        .collect();
    assert_eq!(listed, kchess_domain::misc::TV_CHANNEL_KEYS);
}
