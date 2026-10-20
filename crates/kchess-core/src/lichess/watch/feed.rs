//! Decoding the TV and game feeds and broadcast PGN, all pure (moved from `watch.rs` without
//! changes).
// Feed data is outside input: read it without indexing that can panic.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

use serde_json::{Value, json};

use super::{BroadcastGame, MAX_BROADCAST_PGN, VARIANTS, WatchClock, WatchPlayer};
use crate::lichess::reviews::split_whitespace;
use kchess_domain::{api, js, position, review};

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
    text == "-" || matches!(text.as_bytes(), [b'a'..=b'h', b'3' | b'6'])
}

/// A move the feed named: UCI, or a drop (`P@e4`); anything else is not kept.
pub(super) fn uci_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let square = |file: u8, rank: u8| matches!(file, b'a'..=b'h') && matches!(rank, b'1'..=b'8');
    let kept = match *text.as_bytes() {
        [f1, r1, f2, r2] => {
            (square(f1, r1) && square(f2, r2))
                // A drop: `P@e4`.
                || (matches!(f1, b'P' | b'N' | b'B' | b'R' | b'Q') && r1 == b'@' && square(f2, r2))
        }
        [f1, r1, f2, r2, promotion] => {
            square(f1, r1)
                && square(f2, r2)
                && matches!(promotion, b'q' | b'r' | b'b' | b'n' | b'k')
        }
        _ => false,
    };
    kept.then(|| text.to_string())
}

/// The board and side to move of a FEN: what the feed's positions and a replay compare on.
pub(super) fn position_key(fen: &str) -> String {
    let mut spaces = 0;
    for (index, byte) in fen.bytes().enumerate() {
        if byte == b' ' {
            spaces += 1;
            if spaces == 2 {
                return fen[..index].to_string();
            }
        }
    }
    fen.to_string()
}

/// A player record of the feed (`playerOf`): a name, title and rating, or `?` when unreadable.
pub(super) fn player_of(raw: Option<&Value>) -> WatchPlayer {
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
    for (ply, key) in keys.iter().enumerate().rev() {
        let Some(seen) = feed.iter().rposition(|seen| seen == key) else {
            continue;
        };
        let since = live.get(seen..).unwrap_or(&[]);
        let since_moves: Option<Vec<String>> = since.iter().cloned().collect();
        let since_moves = since_moves?;
        let mut moves: Vec<String> = exported.iter().take(ply).cloned().collect();
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
    let (_, id) = url?.rsplit_once('/')?;
    (id.len() == 8 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())).then(|| id.to_string())
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
pub(super) fn take_utf16(text: &str, units: usize) -> String {
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
