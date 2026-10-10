//! Watching Lichess games (`crates/kchess-node/js/spectate.ts`): TV channels and the games they show,
//! a game by id, broadcast rounds and the broadcast listings.
//!
//! One watch runs at a time. Starting another stops the last, and frames or states of a stopped
//! session are never reported. Frames go to a `WatchSink` (`HostSink` emits `watch:state`,
//! `watch:frame` and `watch:broadcast` on the host). Lining a TV game up with its delayed export
//! runs in the background and fails quietly: the board works without it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, path_segment};
use super::reviews::split_whitespace;
use crate::error::{CoreError, Result as CoreResult};
use crate::host::{Host, Level};
use kchess_domain::{api, js, position, review};

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
}

impl HostSink {
    pub fn new(host: Arc<dyn Host>) -> HostSink {
        HostSink { host }
    }
}

impl WatchSink for HostSink {
    fn frame(&self, frame: WatchFrame) {
        self.host.emit(
            "watch:frame",
            serde_json::to_value(frame).unwrap_or(Value::Null),
        );
    }

    fn state(&self, state: WatchState) {
        self.host.emit(
            "watch:state",
            serde_json::to_value(state).unwrap_or(Value::Null),
        );
    }

    fn broadcast(&self, update: BroadcastUpdate) {
        self.host.emit(
            "watch:broadcast",
            serde_json::to_value(update).unwrap_or(Value::Null),
        );
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

/* ── Feed decoding (pure) ── */

/// `displayFen(raw)`: the board and side to move only; Crazyhouse pockets and other fields are
/// dropped. None when the placement is not eight ranks of known squares.
pub fn display_fen(raw: &str) -> Option<String> {
    let parts: Vec<&str> = split_whitespace(js::trim(raw));
    let board = parts.first().copied().unwrap_or("");
    let turn = parts.get(1).copied().unwrap_or("w");
    let castling = parts.get(2).copied().unwrap_or("-");
    let ep = parts.get(3).copied().unwrap_or("-");
    let placement = strip_trailing_pocket(board);
    let ranks: Vec<&str> = placement.split('/').take(8).collect();
    if ranks.len() != 8 || !ranks.iter().all(|rank| is_rank(rank)) {
        return None;
    }
    let turn = if turn == "b" { "b" } else { "w" };
    let castling = if is_castling(castling) { castling } else { "-" };
    let ep = if is_en_passant(ep) { ep } else { "-" };
    Some(format!(
        "{} {turn} {castling} {ep} 0 1",
        ranks.join("/").replace('~', "")
    ))
}

/// `/\[[^\]]*\]$/` removed from the end of the placement.
fn strip_trailing_pocket(board: &str) -> &str {
    if let Some(stripped) = board.strip_suffix(']')
        && let Some(open) = stripped.rfind('[')
        && !stripped[open..].contains(']')
    {
        return &board[..open];
    }
    board
}

/// `/^[1-8pnbrqkPNBRQK~]{1,16}$/`
fn is_rank(rank: &str) -> bool {
    (1..=16).contains(&rank.len())
        && rank.chars().all(|c| {
            matches!(
                c,
                '1'..='8'
                    | 'p'
                    | 'n'
                    | 'b'
                    | 'r'
                    | 'q'
                    | 'k'
                    | 'P'
                    | 'N'
                    | 'B'
                    | 'R'
                    | 'Q'
                    | 'K'
                    | '~'
            )
        })
}

/// `/^[KQkqA-Ha-h]{1,4}$|^-$/`
fn is_castling(text: &str) -> bool {
    text == "-"
        || ((1..=4).contains(&text.len())
            && text
                .chars()
                .all(|c| matches!(c, 'K' | 'Q' | 'k' | 'q' | 'A'..='H' | 'a'..='h')))
}

/// `/^[a-h][36]$|^-$/`
fn is_en_passant(text: &str) -> bool {
    text == "-"
        || (text.len() == 2
            && matches!(text.as_bytes()[0], b'a'..=b'h')
            && matches!(text.as_bytes()[1], b'3' | b'6'))
}

/// A move the feed named: UCI, or a drop (`P@e4`); anything else is not kept.
fn uci_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let bytes = text.as_bytes();
    let square = |b: &[u8]| matches!(b[0], b'a'..=b'h') && matches!(b[1], b'1'..=b'8');
    let plain = bytes.len() >= 4
        && bytes.len() <= 5
        && square(&bytes[0..2])
        && square(&bytes[2..4])
        && (bytes.len() == 4 || matches!(bytes[4], b'q' | b'r' | b'b' | b'n' | b'k'));
    let drop = bytes.len() == 4
        && matches!(bytes[0], b'P' | b'N' | b'B' | b'R' | b'Q')
        && bytes[1] == b'@'
        && square(&bytes[2..4]);
    (plain || drop).then(|| text.to_string())
}

/// The board and side to move of a FEN: what the feed's positions and a replay compare on.
fn position_key(fen: &str) -> String {
    fen.split(' ').take(2).collect::<Vec<_>>().join(" ")
}

/// A player record of the feed (`playerOf`): a name, title and rating, or `?` when unreadable.
fn player_of(raw: Option<&Value>) -> WatchPlayer {
    let unreadable = || WatchPlayer {
        name: "?".into(),
        ..WatchPlayer::default()
    };
    let Some(raw) = raw.filter(|value| value.is_object()) else {
        return unreadable();
    };
    // A text field of at most `max` UTF-16 units; absent or null is fine, anything else is not.
    let text = |value: Option<&Value>, max: usize| -> Option<Option<String>> {
        match value {
            None | Some(Value::Null) => Some(None),
            Some(Value::String(text)) if js::utf16_len(text) <= max => Some(Some(text.clone())),
            Some(_) => None,
        }
    };
    let rating = match raw.get("rating") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_f64() {
            Some(rating) if rating.is_finite() => Some(rating),
            _ => return unreadable(),
        },
    };
    let ai_level = match raw.get("aiLevel") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_f64() {
            Some(level) if level.is_finite() => Some(level),
            _ => return unreadable(),
        },
    };
    let (user_name, user_title) = match raw.get("user") {
        None | Some(Value::Null) => (None, None),
        Some(user) => {
            let (Some(name), Some(_), Some(title)) = (
                text(user.get("name"), 40),
                text(user.get("id"), 40),
                text(user.get("title"), 8),
            ) else {
                return unreadable();
            };
            (name, title)
        }
    };
    let Some(name) = text(raw.get("name"), 40) else {
        return unreadable();
    };
    let name = user_name.or(name).unwrap_or_else(|| match ai_level {
        // `aiLevel ? ... : 'Anonymous'`: zero is no level.
        Some(level) if level != 0.0 => format!("Stockfish level {}", js::number_to_string(level)),
        _ => "Anonymous".into(),
    });
    WatchPlayer {
        name,
        title: user_title,
        rating,
    }
}

/// The details the TV feed leaves out: rated flag, speed and time control (`parseGameDetails`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameDetails {
    pub rated: Option<bool>,
    pub speed: Option<String>,
    pub clock: Option<WatchClock>,
}

/// `parseGameDetails(raw)`: each field kept when it has the right type; an unreadable clock or
/// speed drops all details.
pub fn parse_game_details(raw: &Value) -> GameDetails {
    let rated = match raw.get("rated") {
        None => None,
        Some(Value::Bool(flag)) => Some(*flag),
        Some(_) => return GameDetails::default(),
    };
    let speed = match raw.get("speed") {
        None => None,
        Some(Value::String(text)) if js::utf16_len(text) <= 20 => Some(text.clone()),
        Some(_) => return GameDetails::default(),
    };
    let clock = match raw.get("clock") {
        None => None,
        Some(clock) => {
            let number = |key: &str| {
                clock
                    .get(key)
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite())
            };
            match (number("initial"), number("increment")) {
                (Some(initial), Some(increment)) => Some(WatchClock { initial, increment }),
                _ => return GameDetails::default(),
            }
        }
    };
    GameDetails {
        rated,
        speed,
        clock,
    }
}

/// A game's details, start and SAN moves from its JSON export (`TvGame`).
#[derive(Clone, Debug, PartialEq)]
pub struct TvGame {
    pub details: GameDetails,
    /// Absent for variants KChess cannot replay (Crazyhouse).
    pub setup: Option<GameSetup>,
    pub san: Vec<String>,
    pub status: Option<String>,
    pub winner: Option<&'static str>,
}

/// The rules and the position before the first move (`GameSetup`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSetup {
    pub variant: &'static str,
    pub fen: String,
}

/// `variantFromLichess(key)`: the playable variant of a Lichess key; `fromPosition` is standard.
pub fn variant_from_lichess(key: Option<&str>) -> Option<&'static str> {
    match key {
        None | Some("") | Some("fromPosition") => Some("standard"),
        Some(key) => VARIANTS.iter().copied().find(|variant| *variant == key),
    }
}

/// `defaultFen(variant)`: the usual start of a variant.
fn default_fen(variant: &str) -> Option<String> {
    domain_call("defaultFen", json!([variant]))?
        .as_str()
        .map(str::to_string)
}

/// Calls a domain method that answers in JSON.
fn domain_call(method: &str, args: Value) -> Option<Value> {
    let text = api::call(method, &args.to_string()).ok()?;
    serde_json::from_str(&text).ok()
}

/// `parseTvGame(raw)`: the start, moves and result of a game export. A malformed export keeps
/// its details and gets no moves.
pub fn parse_tv_game(raw: &Value) -> TvGame {
    let details = parse_game_details(raw);
    // (moves, initialFen, variant, status, winner) when the export reads as a whole.
    type Fields<'a> = (
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
    );
    let parsed = (|| -> Option<Fields> {
        let text = |key: &str, max: usize| -> Option<Option<&str>> {
            match raw.get(key) {
                None => Some(None),
                Some(Value::String(text)) if js::utf16_len(text) <= max => {
                    Some(Some(text.as_str()))
                }
                Some(_) => None,
            }
        };
        let moves = text("moves", 20_000)?;
        let initial = text("initialFen", 120)?;
        let variant = text("variant", 20)?;
        let status = text("status", 30)?;
        let winner = match raw.get("winner") {
            None => None,
            Some(Value::String(text)) if text == "white" || text == "black" => Some(text.as_str()),
            Some(_) => return None,
        };
        Some((moves, initial, variant, status, winner))
    })();
    let Some((moves, initial, variant, status, winner)) = parsed else {
        return TvGame {
            details,
            setup: None,
            san: Vec::new(),
            status: None,
            winner: None,
        };
    };
    let variant = variant_from_lichess(Some(variant.unwrap_or("standard")));
    let setup = variant.and_then(|variant| {
        let fen = match initial {
            Some(fen) => Some(fen.to_string()),
            None => default_fen(variant),
        }?;
        Some(GameSetup { variant, fen })
    });
    TvGame {
        details,
        setup,
        san: moves
            .map(|moves| {
                moves
                    .split(' ')
                    .filter(|token| !token.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        status: status.map(str::to_string),
        winner: winner.map(|color| if color == "white" { "white" } else { "black" }),
    }
}

/// `alignTvMoves(setup, san, feed, live)`: the export's moves joined to the moves the feed sent
/// after it. `feed` holds the positions the feed showed, and `live[i]` is the move from `feed[i]`
/// to `feed[i + 1]`. Undefined until the export reaches a position the feed sent.
pub fn align_tv_moves(
    setup: &GameSetup,
    san: &[String],
    feed: &[String],
    live: &[Option<String>],
) -> Option<Vec<String>> {
    let setup_json = json!({ "variant": setup.variant, "fen": setup.fen });
    let start = domain_call("line", json!([setup_json, [], { "san": true }]))?;
    let start_fen = start.get("start")?.as_str()?.to_string();
    if feed.is_empty() {
        return None;
    }
    let line = domain_call("line", json!([setup_json, san, { "san": true }]))?;
    let line = line.get("moves")?.as_array()?.clone();
    if line.len() < san.len() {
        return None;
    }
    let exported: Vec<String> = line
        .iter()
        .filter_map(|move_| move_.get("uci").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut keys = vec![position_key(&start_fen)];
    keys.extend(
        line.iter()
            .filter_map(|move_| move_.get("fen").and_then(Value::as_str))
            .map(position_key),
    );
    let target = position_key(feed.last()?);
    for ply in (0..keys.len()).rev() {
        let Some(seen) = feed.iter().rposition(|key| *key == keys[ply]) else {
            continue;
        };
        let since = live.get(seen..).unwrap_or(&[]);
        let since_moves: Option<Vec<String>> = since.iter().cloned().collect();
        let since_moves = since_moves?;
        let mut moves: Vec<String> = exported[..ply.min(exported.len())].to_vec();
        moves.extend(since_moves);
        let replayed = replay_uci(setup, &moves)?;
        if replayed.0 != moves.len() {
            return None;
        }
        return (position_key(&replayed.1) == target).then_some(moves);
    }
    None
}

/// `replaySetup(setup, moves)`: how many UCI moves replay and the position after them.
fn replay_uci(setup: &GameSetup, moves: &[String]) -> Option<(usize, String)> {
    let replay = kchess_domain::replay::replay_setup(setup.variant, &setup.fen, moves)?;
    Some((replay.played.len(), replay.fen))
}

/// `broadcastGame(pgn)`: one broadcast board, with the main line replayed and checked.
pub fn broadcast_game(pgn: &str) -> Option<BroadcastGame> {
    let text = take_utf16(pgn, MAX_BROADCAST_PGN);
    let game = position::pgn_mainline(&text, 1001);
    let start_fen = game.get("start")?.as_str()?.to_string();
    let headers = header_map(game.get("headers"));
    let header = |name: &str| {
        headers
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let moves_json = game.get("moves")?.as_array()?;
    let moves: Vec<String> = moves_json
        .iter()
        .filter_map(|entry| entry.get("uci").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut white_clock = None;
    let mut black_clock = None;
    for entry in moves_json {
        let Some(clock) = entry.get("clock").and_then(Value::as_f64) else {
            continue;
        };
        let rounded = Some(review::js_round(clock));
        if entry.get("mover").and_then(Value::as_str) == Some("white") {
            white_clock = rounded;
        } else {
            black_clock = rounded;
        }
    }
    let chapter = last_path_id(header("ChapterURL"))
        .or_else(|| last_path_id(header("GameURL")))
        .unwrap_or_else(|| {
            format!(
                "{}-{}-{}",
                header("White").unwrap_or(""),
                header("Black").unwrap_or(""),
                header("Board").unwrap_or("")
            )
        });
    let result = header("Result").unwrap_or("*").to_string();
    let fen = moves_json
        .last()
        .and_then(|entry| entry.get("fen").and_then(Value::as_str))
        .unwrap_or(&start_fen)
        .to_string();
    let rating = |value: Option<&str>| -> Option<f64> {
        let number = js_number(value.unwrap_or(""));
        (number.is_finite() && number > 0.0).then_some(number)
    };
    Some(BroadcastGame {
        id: take_utf16(&chapter, 64),
        name: take_utf16(header("Event").unwrap_or(""), 120),
        white: WatchPlayer {
            name: take_utf16(header("White").unwrap_or("White"), 60),
            title: header("WhiteTitle").map(|title| take_utf16(title, 8)),
            rating: rating(header("WhiteElo")),
        },
        black: WatchPlayer {
            name: take_utf16(header("Black").unwrap_or("Black"), 60),
            title: header("BlackTitle").map(|title| take_utf16(title, 8)),
            rating: rating(header("BlackElo")),
        },
        ongoing: result == "*",
        result,
        start_fen,
        last_move: moves.last().cloned(),
        moves,
        fen,
        white_clock,
        black_clock,
        pgn: take_utf16(pgn, MAX_BROADCAST_PGN),
    })
}

/// `headers` of `pgnMainline` as pairs.
fn header_map(raw: Option<&Value>) -> Vec<(String, String)> {
    raw.and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    Some((
                        pair.first()?.as_str()?.to_string(),
                        pair.get(1)?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `/\/([a-zA-Z0-9]{8})$/` on a broadcast URL: the chapter id.
fn last_path_id(url: Option<&str>) -> Option<String> {
    let bytes = url?.as_bytes();
    let n = bytes.len();
    (n >= 9 && bytes[n - 9] == b'/' && bytes[n - 8..].iter().all(u8::is_ascii_alphanumeric))
        .then(|| String::from_utf8_lossy(&bytes[n - 8..]).into_owned())
}

/// `Number(text)`: JavaScript's conversion (blank is zero, unreadable is NaN).
fn js_number(text: &str) -> f64 {
    let trimmed = js::trim(text);
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// The first `units` UTF-16 units of `text`.
fn take_utf16(text: &str, units: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let width = c.len_utf16();
        if used + width > units {
            break;
        }
        used += width;
        out.push(c);
    }
    out
}

/* ── Broadcast listings (pure) ── */

/// `broadcastSummaries(entries, section)`: the listing's entries that have a readable tournament;
/// only Lichess's own images are kept. A round's section follows its state.
pub fn broadcast_summaries(entries: &[Value], section: &'static str) -> Vec<BroadcastSummary> {
    entries
        .iter()
        .take(30)
        .filter_map(|entry| {
            let tour = entry.get("tour")?;
            let tour = read_tour(tour)?;
            let round = match entry.get("round") {
                None | Some(Value::Null) => None,
                Some(round) => Some(read_round(round)?),
            };
            let ongoing = round.as_ref().is_some_and(|round| round.ongoing);
            let upcoming = round.as_ref().is_some_and(|round| {
                !ongoing && !round.finished && round.finished_at.is_none_or(|at| at == 0.0)
            });
            let section = if ongoing {
                "active"
            } else if upcoming {
                "upcoming"
            } else {
                section
            };
            Some(BroadcastSummary {
                players: tour.players,
                tour_id: tour.id,
                tour_name: tour.name,
                description: tour.description,
                image: tour
                    .image
                    .filter(|image| image.starts_with("https://image.lichess1.org/")),
                round_id: round.as_ref().map(|round| round.id.clone()),
                round_name: round.as_ref().map(|round| round.name.clone()),
                ongoing: round.as_ref().is_some_and(|round| round.ongoing),
                starts_at: round.as_ref().and_then(|round| round.starts_at),
                section,
            })
        })
        .collect()
}

struct TourRaw {
    id: String,
    name: String,
    description: Option<String>,
    image: Option<String>,
    players: Option<String>,
}

struct RoundRaw {
    id: String,
    name: String,
    ongoing: bool,
    finished: bool,
    finished_at: Option<f64>,
    starts_at: Option<i64>,
}

/// A text field of a tournament or round: absent, or a string of at most `max` UTF-16 units.
fn text_field(raw: &Value, key: &str, max: usize) -> Option<Option<String>> {
    match raw.get(key) {
        None | Some(Value::Null) if key != "id" && key != "name" => Some(None),
        Some(Value::String(text)) if js::utf16_len(text) <= max => Some(Some(text.clone())),
        _ => None,
    }
}

fn read_tour(raw: &Value) -> Option<TourRaw> {
    let id = text_field(raw, "id", 12)??;
    let name = text_field(raw, "name", 200)??;
    let description = text_field(raw, "description", 4000)?;
    let image = text_field(raw, "image", 400)?;
    let players = match raw.get("info") {
        None | Some(Value::Null) => None,
        Some(info) => text_field(info, "players", 500)?,
    };
    Some(TourRaw {
        id,
        name,
        description,
        image,
        players,
    })
}

fn read_round(raw: &Value) -> Option<RoundRaw> {
    let id = text_field(raw, "id", 12)??;
    let name = text_field(raw, "name", 120)??;
    let flag = |key: &str| -> Option<bool> {
        match raw.get(key) {
            None | Some(Value::Null) => Some(false),
            Some(Value::Bool(flag)) => Some(*flag),
            Some(_) => None,
        }
    };
    let number = |key: &str| -> Option<Option<f64>> {
        match raw.get(key) {
            None | Some(Value::Null) => Some(None),
            Some(value) => value.as_f64().map(Some),
        }
    };
    Some(RoundRaw {
        id,
        name,
        ongoing: flag("ongoing")?,
        finished: flag("finished")?,
        finished_at: number("finishedAt")?,
        starts_at: number("startsAt")?.map(|n| n as i64),
    })
}

/* ── Requests ── */

/// `tvChannels()`: the TV channels with who is playing on each right now.
pub async fn tv_channels(lichess: &Lichess) -> CoreResult<Vec<TvChannel>> {
    let cancel = lichess.lifetime().clone();
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess
                .client()
                .get_json("/api/tv/channels", &[], None, &cancel)
                .await
        })
        .await?;
    Ok(TV_CHANNELS
        .iter()
        .map(|(key, label)| {
            let entry = raw.get(*key);
            let user = entry.and_then(|entry| entry.get("user"));
            let player = user
                .and_then(|user| user.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(|name| WatchPlayer {
                    name: name.to_string(),
                    title: user
                        .and_then(|user| user.get("title"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    rating: entry
                        .and_then(|entry| entry.get("rating"))
                        .and_then(Value::as_f64),
                });
            TvChannel {
                key,
                label,
                game_id: entry
                    .and_then(|entry| entry.get("gameId"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                player,
            }
        })
        .collect())
}

/// `broadcasts(query)`: the search results for a query, or the broadcasts Lichess features: live,
/// upcoming, then recently finished.
pub async fn broadcasts(
    lichess: &Lichess,
    query: Option<&str>,
) -> CoreResult<Vec<BroadcastSummary>> {
    let cancel = lichess.lifetime().clone();
    let query = query.map(js::trim).filter(|q| !q.is_empty());
    if let Some(query) = query {
        let raw: Value = lichess
            .client()
            .policy()
            .with_usage("", "watch", async {
                lichess
                    .client()
                    .get_json(
                        "/api/broadcast/search",
                        &[("q", query.to_string()), ("page", "1".to_string())],
                        None,
                        &cancel,
                    )
                    .await
            })
            .await?;
        let entries = raw
            .get("currentPageResults")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(broadcast_summaries(&entries, "past"));
    }
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess
                .client()
                .get_json(
                    "/api/broadcast/top",
                    &[("page", "1".to_string())],
                    None,
                    &cancel,
                )
                .await
        })
        .await?;
    let list = |key: &str| {
        raw.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let past = raw
        .pointer("/past/currentPageResults")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut all = broadcast_summaries(&list("active"), "active");
    all.extend(broadcast_summaries(&list("upcoming"), "upcoming"));
    all.extend(broadcast_summaries(&past, "past"));
    Ok(all)
}

/// `broadcastTour(id)`: a broadcast tournament and its rounds (at most 100).
pub async fn broadcast_tour(lichess: &Lichess, id: &str) -> CoreResult<BroadcastTourDetail> {
    let cancel = lichess.lifetime().clone();
    let path = format!("/api/broadcast/{}", path_segment(id));
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "watch", async {
            lichess.client().get_json(&path, &[], None, &cancel).await
        })
        .await?;
    let unreadable = || CoreError::new("Lichess sent an unreadable broadcast.");
    let tour = raw.get("tour").and_then(read_tour).ok_or_else(unreadable)?;
    let rounds = raw
        .get("rounds")
        .and_then(Value::as_array)
        .map(|rounds| {
            rounds
                .iter()
                .take(100)
                .filter_map(read_round)
                .map(|round| BroadcastRoundRef {
                    id: round.id,
                    name: round.name,
                    ongoing: round.ongoing,
                    finished: round.finished,
                    starts_at: round.starts_at,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(BroadcastTourDetail {
        id: tour.id,
        name: tour.name,
        description: tour.description,
        rounds,
        default_round_id: raw
            .get("defaultRoundId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

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
            let result = lichess
                .client()
                .policy()
                .with_usage("", "watch", async {
                    match &target {
                        WatchTarget::Channel(channel) => {
                            inner.follow_tv(channel, session, &token).await
                        }
                        WatchTarget::Game(id) => inner.follow_game(id, session, &token).await,
                    }
                })
                .await;
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
                        cause.message.clone()
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
            let result = lichess
                .client()
                .policy()
                .with_usage("", "watch", inner.follow_round(&round_id, session, &token))
                .await;
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
                            cause.message.clone()
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
        let live_for_lines = Arc::clone(&live);
        let token_for_lines = token.clone();
        let inner = Arc::clone(self);
        let channel_owned = channel.to_string();
        self.lichess
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
            .await?;
        Ok(())
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
        let data = message.get("d").cloned().unwrap_or_else(|| json!({}));
        let fen_of =
            |data: &Value| display_fen(data.get("fen").and_then(Value::as_str).unwrap_or(""));
        match message.get("t").and_then(Value::as_str) {
            Some("featured") => {
                let players = data
                    .get("players")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let side = |color: &str| {
                    players
                        .iter()
                        .find(|player| player.get("color").and_then(Value::as_str) == Some(color))
                        .cloned()
                };
                let Some(fen) = fen_of(&data) else {
                    return Ok(());
                };
                let Some(id) = data.get("id").and_then(Value::as_str) else {
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
                    orientation: if data.get("orientation").and_then(Value::as_str) == Some("black")
                    {
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
            Some("fen") => {
                let Some(fen) = fen_of(&data) else {
                    return Ok(());
                };
                let last_move = uci_or_none(data.get("lm"));
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
                    white_clock: data
                        .get("wc")
                        .and_then(Value::as_f64)
                        .or(current.white_clock),
                    black_clock: data
                        .get("bc")
                        .and_then(Value::as_f64)
                        .or(current.black_clock),
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
            _ => Ok(()),
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
        let path = format!("/game/export/{}", path_segment(id));
        let query = [
            ("moves", "true".to_string()),
            ("tags", "false".to_string()),
            ("clocks", "false".to_string()),
            ("evals", "false".to_string()),
            ("opening", "false".to_string()),
        ];
        let raw: Value = self
            .lichess
            .client()
            .get_json(&path, &query, None, token)
            .await
            .map_err(CoreError::from)?;
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
                    let Some(fen) =
                        display_fen(data.get("fen").and_then(Value::as_str).unwrap_or(""))
                    else {
                        return Ok(());
                    };
                    if let Some(id) = data.get("id").and_then(Value::as_str) {
                        // The description of the game: first, and again when it ends.
                        let status = data
                            .pointer("/status/name")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let players = data.get("players").cloned().unwrap_or_else(|| json!({}));
                        let winner = match data.get("winner").and_then(Value::as_str) {
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
                            last_move: uci_or_none(data.get("lastMove")),
                            white_clock: current.as_ref().and_then(|c| c.white_clock),
                            black_clock: current.as_ref().and_then(|c| c.black_clock),
                            variant: data
                                .pointer("/variant/key")
                                .and_then(Value::as_str)
                                .unwrap_or("standard")
                                .to_string(),
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
                            last_move: uci_or_none(data.get("lm")),
                            white_clock: data
                                .get("wc")
                                .and_then(Value::as_f64)
                                .or(previous.white_clock),
                            black_clock: data
                                .get("bc")
                                .and_then(Value::as_f64)
                                .or(previous.black_clock),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_board_and_side_to_move() {
        assert_eq!(
            display_fen("rnbqkb1r/ppp1pppp/5n2/3p4/3P4/2N5/PPP1PPPP/R1BQKBNR[] w KQkq - 2 3")
                .as_deref(),
            Some("rnbqkb1r/ppp1pppp/5n2/3p4/3P4/2N5/PPP1PPPP/R1BQKBNR w KQkq - 0 1")
        );
        assert_eq!(display_fen("rnbqkbnr/pppppppp b"), None);
        assert_eq!(display_fen("8/8/8/8/8/8/8/8<script> w - - 0 1"), None);
    }

    #[test]
    fn keeps_only_known_move_forms() {
        assert_eq!(uci_or_none(Some(&json!("e2e4"))).as_deref(), Some("e2e4"));
        assert_eq!(uci_or_none(Some(&json!("e7e8q"))).as_deref(), Some("e7e8q"));
        assert_eq!(uci_or_none(Some(&json!("P@e4"))).as_deref(), Some("P@e4"));
        assert_eq!(uci_or_none(Some(&json!("e2e4x"))), None);
    }

    #[test]
    fn the_tv_feed_names_players_without_inventing_ratings() {
        assert_eq!(player_of(None).name, "?");
        let engine = json!({ "aiLevel": 3 });
        assert_eq!(player_of(Some(&engine)).name, "Stockfish level 3");
        let anonymous = json!({ "rating": 1500 });
        assert_eq!(
            player_of(Some(&anonymous)),
            WatchPlayer {
                name: "Anonymous".into(),
                title: None,
                rating: Some(1500.0),
            }
        );
    }

    #[test]
    fn variants_follow_the_lichess_keys_kchess_plays() {
        assert_eq!(variant_from_lichess(None), Some("standard"));
        assert_eq!(variant_from_lichess(Some("fromPosition")), Some("standard"));
        assert_eq!(variant_from_lichess(Some("chess960")), Some("chess960"));
        assert_eq!(variant_from_lichess(Some("crazyhouse")), None);
    }
}
