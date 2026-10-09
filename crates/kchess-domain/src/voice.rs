//! Rules moved from `core/src/domain` (see RUST_MIGRATION.md), reached through `api::call`:
//! spoken commands and moves (`voiceCommands.ts`), the board editor's string logic
//! (`boardEditor.ts`), coordinates (`coordinates.ts`), knight paths (`knight.ts`) and material
//! (`material.ts`). The grammar lists are pinned to the TypeScript constants by
//! `core/tests/unit/native-voice.test.ts`.
//!
//! JavaScript semantics are mirrored deliberately: `toLowerCase`, `\s` splitting, `trim`,
//! `Number(…)` coercion and `String(…)` formatting (see `crate::js`). No regex crate: each
//! pattern below is the hand-written equivalent of the regex it names.

use serde_json::{Value, json};
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position};
use std::collections::HashSet;

use crate::js;
use crate::position::{self, LegalMove};
use crate::rules;

type Result<T> = std::result::Result<T, String>;

const FILES: [char; 8] = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
const RANK_WORDS: [&str; 8] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight",
];
const PHONETICS: [&str; 8] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
];
/// `Object.keys(roles)` in its declaration order.
const ROLE_WORDS: [&str; 9] = [
    "pawn", "pawns", "pond", "knight", "night", "bishop", "rook", "queen", "king",
];
const CONFIRM_WORDS: [&str; 4] = ["confirm", "yes", "yeah", "okay"];
const CANCEL_WORDS: [&str; 2] = ["cancel", "no"];
/// Filler that may accompany a confirmation: “yes play”, “confirm move”.
const CONFIRM_FILLER: [&str; 2] = ["play", "move"];
const REMOVE_WORDS: [&str; 4] = ["remove", "clear", "delete", "empty"];
const GAME_COMMANDS: [(&str, &str); 6] = [
    ("take back", "takeback"),
    ("undo", "takeback"),
    ("new game", "newGame"),
    ("resign", "resign"),
    ("flip board", "flip"),
    ("switch sides", "flip"),
];
const ANALYSIS_COMMANDS: [(&str, &str); 18] = [
    ("back", "back"),
    ("go back", "back"),
    ("previous", "back"),
    ("undo", "back"),
    ("take back", "back"),
    ("forward", "forward"),
    ("next", "forward"),
    ("go forward", "forward"),
    ("first move", "start"),
    ("go to start", "start"),
    ("last move", "end"),
    ("go to end", "end"),
    ("best move", "best"),
    ("play best move", "best"),
    ("flip board", "flip"),
    ("toggle engine", "engine"),
    ("engine on", "engine"),
    ("engine off", "engine"),
];
const EDITOR_PHRASE_KEYS: [&str; 15] = [
    "clear board",
    "empty board",
    "clear the board",
    "starting position",
    "start position",
    "reset board",
    "reset the board",
    "white to move",
    "white to play",
    "black to move",
    "black to play",
    "flip board",
    "flip the board",
    "analyze",
    "analysis",
];

const INITIAL_BOARD: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR";
const EMPTY_BOARD: &str = "8/8/8/8/8/8/8/8";
const EMPTY_FEN: &str = "8/8/8/8/8/8/8/8 w - - 0 1";

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Result<Value>> {
    let result = match method {
        "COORDINATE_GRAMMAR" => Ok(json!(coordinate_grammar())),
        "MOVE_GRAMMAR" => Ok(json!(move_grammar())),
        "GAME_GRAMMAR" => Ok(json!(game_grammar())),
        "EDITOR_GRAMMAR" => Ok(json!(editor_grammar())),
        "ANALYSIS_GRAMMAR" => Ok(json!(analysis_grammar())),
        "FILES" => Ok(json!(FILES.map(|f| f.to_string()))),
        "RANKS" => Ok(json!(["1", "2", "3", "4", "5", "6", "7", "8"])),
        "ALL_SQUARES" => Ok(json!(all_squares())),
        "EMPTY_FEN" => Ok(json!(EMPTY_FEN)),
        "EMPTY_BOARD" => Ok(json!(EMPTY_BOARD)),
        "START_SETUP" => Ok(json!({
            "board": INITIAL_BOARD,
            "turn": "white",
            "castling": { "K": true, "Q": true, "k": true, "q": true },
            "ep": "",
        })),
        "spokenCommand" => text(args, 0).map(|t| opt(game_command(t))),
        "spokenChoice" => text(args, 0).map(spoken_choice),
        "spokenSquare" => text(args, 0).map(|t| opt(spoken_square(t).as_deref())),
        "spokenMove" => spoken_move_call(args),
        "spokenEdit" => text(args, 0)
            .map(|t| spoken_edit(t, args.get(1).and_then(Value::as_str).unwrap_or("white"))),
        "spokenAnalysisCommand" => text(args, 0).map(|t| opt(analysis_command(t))),
        "withPiece" => with_piece_call(args),
        "castlingAvailable" => text(args, 0).map(castling_available_json),
        "enPassantSquares" => text(args, 0)
            .and_then(|board| text(args, 1).map(|turn| json!(en_passant_squares(board, turn)))),
        "setupFen" => setup_fen_call(args),
        "randomSquare" => random_square_call(args),
        "squareColor" => text(args, 0).map(|s| json!(square_color(s))),
        "knightMoves" => text(args, 0).map(|s| json!(knight_moves(s))),
        "knightDistance" => text(args, 0).and_then(|from| {
            text(args, 1).map(|to| knight_distance(from, to).map_or(Value::Null, |n| json!(n)))
        }),
        "knightChallenge" => knight_challenge_call(args),
        "knightFen" => text(args, 0).map(|s| json!(knight_fen(s))),
        "materialBalance" => text(args, 0).map(|fen| json!(material_balance(fen))),
        _ => return None,
    };
    Some(result)
}

fn text(args: &[Value], i: usize) -> Result<&str> {
    args.get(i)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument {i} must be a string"))
}

fn numbers(args: &[Value], i: usize) -> Result<Vec<f64>> {
    args.get(i)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_f64).collect())
        .ok_or_else(|| format!("argument {i} must be a list of numbers"))
}

fn number(args: &[Value], i: usize) -> Result<f64> {
    args.get(i)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("argument {i} must be a number"))
}

fn opt(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |s| json!(s))
}

/* ── Grammar lists ── */

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn all_squares() -> Vec<String> {
    FILES
        .iter()
        .flat_map(|file| (1..=8).map(move |rank| format!("{file}{rank}")))
        .collect()
}

fn coordinate_grammar() -> Vec<String> {
    let mut out = Vec::new();
    for (file_index, file) in FILES.iter().enumerate() {
        for rank in 1..=8usize {
            let rank_word = RANK_WORDS[rank - 1];
            out.push(format!("{file} {rank_word}"));
            out.push(format!("{} {rank_word}", PHONETICS[file_index]));
        }
    }
    out.push("[unk]".into());
    out
}

/// The roles that stay in the move grammar (“night” and “pawns” are left out on purpose).
fn move_role_words() -> Vec<String> {
    ROLE_WORDS
        .iter()
        .filter(|word| !matches!(**word, "night" | "pawns"))
        .map(|word| word.to_string())
        .collect()
}

fn move_grammar() -> Vec<String> {
    let mut out = coordinate_grammar();
    out.extend(move_role_words());
    out.extend(strings(&[
        "to",
        "from",
        "takes",
        "captures",
        "move",
        "play",
        "castle",
        "king side",
        "queen side",
        "promote",
        "promotion",
        "equals",
    ]));
    out.extend(strings(&CONFIRM_WORDS));
    out.extend(strings(&CANCEL_WORDS));
    out.extend(strings(&RANK_WORDS));
    out
}

fn game_grammar() -> Vec<String> {
    let mut out = move_grammar();
    out.extend(GAME_COMMANDS.iter().map(|(phrase, _)| phrase.to_string()));
    out
}

fn editor_grammar() -> Vec<String> {
    let mut out = coordinate_grammar();
    out.extend(move_role_words());
    out.extend(strings(&["white", "black", "on", "to"]));
    out.extend(strings(&REMOVE_WORDS));
    out.extend(strings(&EDITOR_PHRASE_KEYS));
    out
}

fn analysis_grammar() -> Vec<String> {
    let mut out = move_grammar();
    out.extend(
        ANALYSIS_COMMANDS
            .iter()
            .map(|(phrase, _)| phrase.to_string()),
    );
    out
}

/* ── Words ── */

fn file_word(word: &str) -> Option<&'static str> {
    Some(match word {
        "a" | "alpha" | "alfa" | "ay" => "a",
        "b" | "bravo" | "bee" => "b",
        "c" | "charlie" | "see" => "c",
        "d" | "delta" | "dee" => "d",
        "e" | "echo" => "e",
        "f" | "foxtrot" => "f",
        "g" | "golf" | "gee" => "g",
        "h" | "hotel" | "aitch" => "h",
        _ => return None,
    })
}

fn rank_word(word: &str) -> Option<&'static str> {
    Some(match word {
        "one" => "1",
        "two" => "2",
        "three" => "3",
        "four" => "4",
        "five" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        _ => return None,
    })
}

fn role_word(word: &str) -> Option<&'static str> {
    Some(match word {
        "pawn" | "pawns" | "pond" => "pawn",
        "knight" | "night" => "knight",
        "bishop" => "bishop",
        "rook" => "rook",
        "queen" => "queen",
        "king" => "king",
        _ => return None,
    })
}

/// `/^[a-h][1-8]$/`
fn is_square(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && (b'a'..=b'h').contains(&b[0]) && (b'1'..=b'8').contains(&b[1])
}

/// `/^[1-8]$/`
fn is_rank_digit(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 1 && (b'1'..=b'8').contains(&b[0])
}

/// `/^(?:[a-h][1-8]|[a-h]|[1-8])$/`
fn is_source(s: &str) -> bool {
    let b = s.as_bytes();
    is_square(s)
        || (b.len() == 1 && ((b'a'..=b'h').contains(&b[0]) || (b'1'..=b'8').contains(&b[0])))
}

/// `text.toLowerCase().replace(/[.,!?]/g, '').split(/\s+/)`, without empty words or `[unk]`.
fn split_words(text: &str) -> Vec<String> {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '.' | ',' | '!' | '?'))
        .collect();
    cleaned
        .split(js::is_space)
        .filter(|word| !word.is_empty() && *word != "[unk]")
        .map(String::from)
        .collect()
}

/// The words joined by single spaces, as the command lookups compare them.
fn phrase(words: &[String]) -> String {
    words.join(" ")
}

/// The spoken words with the letter “a” and rank words resolved to squares.
///
/// The letter “a” said “ay” is often heard as “eight”: “bishop a three” reads as
/// “bishop eight three”. Rank-then-rank is never a square, so it reads as file a, unless a
/// square follows (“rook eight to a three”, where the eight is a source rank).
fn tokens(text: &str) -> Vec<String> {
    let words = split_words(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].as_str();
        let next = words.get(i + 1).map(String::as_str);
        let after = words.get(i + 2).map(String::as_str);
        let file = if word == "eight"
            && next.and_then(rank_word).is_some()
            && after.and_then(file_word).is_none()
        {
            Some("a")
        } else {
            file_word(word)
        };
        let rank = next
            .and_then(rank_word)
            .map(String::from)
            .or_else(|| next.filter(|n| is_rank_digit(n)).map(String::from));
        match (file, rank) {
            (Some(file), Some(rank)) => {
                out.push(format!("{file}{rank}"));
                i += 2;
            }
            (file, _) => {
                out.push(
                    file.map(String::from)
                        .or_else(|| rank_word(word).map(String::from))
                        .unwrap_or_else(|| word.to_string()),
                );
                i += 1;
            }
        }
    }
    out
}

/* ── Commands and choices ── */

fn game_command(text: &str) -> Option<&'static str> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    GAME_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

fn analysis_command(text: &str) -> Option<&'static str> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    ANALYSIS_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

fn spoken_choice(text: &str) -> Value {
    let normalized = js::trim(text).to_lowercase();
    let rank = rank_word(&normalized).map_or(normalized.clone(), String::from);
    if is_rank_digit(&rank) {
        json!(rank.as_bytes()[0] - b'0')
    } else {
        Value::Null
    }
}

fn spoken_square(text: &str) -> Option<String> {
    let words = tokens(text);
    match words.as_slice() {
        [only] if is_square(only) => Some(only.clone()),
        _ => None,
    }
}

/// The board editor's whole-phrase actions (`EDITOR_PHRASES`).
fn editor_phrase(key: &str) -> Option<Value> {
    Some(match key {
        "clear board" | "empty board" | "clear the board" => json!({ "kind": "clear" }),
        "starting position" | "start position" | "reset board" | "reset the board" => {
            json!({ "kind": "start" })
        }
        "white to move" | "white to play" => json!({ "kind": "turn", "color": "white" }),
        "black to move" | "black to play" => json!({ "kind": "turn", "color": "black" }),
        "flip board" | "flip the board" => json!({ "kind": "flip" }),
        "analyze" | "analysis" => json!({ "kind": "analyze" }),
        _ => return None,
    })
}

/// “White knight F three”, “black king on E eight”, “remove E four”, “clear board”, … A piece
/// without a colour takes `color`, the last one used.
fn spoken_edit(text: &str, color: &str) -> Value {
    if let Some(fixed) = editor_phrase(&phrase(&split_words(text))) {
        return fixed;
    }
    let mut words: Vec<String> = tokens(text)
        .into_iter()
        .filter(|w| !matches!(w.as_str(), "on" | "to" | "at" | "the"))
        .collect();
    let Some(square) = words.pop() else {
        return Value::Null;
    };
    if !is_square(&square) {
        return Value::Null;
    }
    let mut rest = words;
    if rest.len() == 1 && REMOVE_WORDS.contains(&rest[0].as_str()) {
        return json!({ "kind": "remove", "square": square });
    }
    let mut spoken_color: Option<String> = None;
    if matches!(rest.first().map(String::as_str), Some("white" | "black")) {
        spoken_color = Some(rest.remove(0));
    }
    match rest.as_slice() {
        [piece] => match role_word(piece) {
            Some(role) => json!({
                "kind": "place",
                "square": square,
                "color": spoken_color.unwrap_or_else(|| color.to_string()),
                "role": role,
            }),
            None => Value::Null,
        },
        _ => Value::Null,
    }
}

/* ── Moves ── */

/// The position a `{ variant, fen }` setup describes; an error when it is not one.
fn setup_position(args: &[Value], i: usize) -> Result<VariantPosition> {
    let setup = args
        .get(i)
        .and_then(Value::as_object)
        .ok_or("setup must be an object")?;
    let variant = setup
        .get("variant")
        .and_then(Value::as_str)
        .ok_or("setup.variant")?;
    let fen = setup
        .get("fen")
        .and_then(Value::as_str)
        .ok_or("setup.fen")?;
    rules::setup_start(variant, fen).ok_or_else(|| "setup is not a legal position".into())
}

fn spoken_move_call(args: &[Value]) -> Result<Value> {
    let text = text(args, 0)?;
    let pos = setup_position(args, 1)?;
    Ok(spoken_move(text, &pos))
}

fn promotion_letter(role: &str) -> Option<&'static str> {
    Some(match role {
        "queen" => "q",
        "rook" => "r",
        "bishop" => "b",
        "knight" => "n",
        _ => return None,
    })
}

/// What a spoken move may be asked to match, once its words are read.
#[derive(Clone, Copy)]
struct Spoken<'a> {
    castle_side: Option<&'static str>,
    dest: Option<&'a str>,
    role: Option<&'static str>,
    source: Option<&'a str>,
    capture: bool,
    promotion: Option<&'static str>,
}

/// Every legal move that matches `spoken`, as `(uci, san)`, in the rules' move order.
fn matching_moves(pos: &VariantPosition, spoken: Spoken) -> Vec<(String, String)> {
    let white = pos.turn() == Color::White;
    let mut choices: Vec<(String, String)> = Vec::new();
    for LegalMove {
        from,
        to,
        role,
        promotion: promoted,
        san,
    } in position::legal_moves(pos)
    {
        if let Some(side) = spoken.castle_side {
            let bare = if san.ends_with('+') || san.ends_with('#') {
                &san[..san.len() - 1]
            } else {
                san.as_str()
            };
            let wanted = if side == "king" { "O-O" } else { "O-O-O" };
            if bare != wanted {
                continue;
            }
        } else {
            let castle_dest = san.starts_with("O-O").then(|| {
                format!(
                    "{}{}",
                    if san.starts_with("O-O-O") { 'c' } else { 'g' },
                    if white { '1' } else { '8' }
                )
            });
            let to_differs = Some(to.as_str()) != spoken.dest;
            let castle_differs = castle_dest.as_deref() != spoken.dest;
            if (to_differs && castle_differs) || spoken.role.is_some_and(|r| r != role) {
                continue;
            }
            if spoken.source.is_some_and(|source| !from.contains(source)) {
                continue;
            }
            if spoken.capture && !san.contains('x') {
                continue;
            }
            if spoken.promotion.is_some_and(|p| promoted != Some(p)) {
                continue;
            }
        }
        // The rules write castling king-takes-rook (e1h1); engines expect standard UCI (e1g1).
        let uci = if san.starts_with("O-O") {
            format!(
                "{from}{}{}",
                if san.starts_with("O-O-O") { 'c' } else { 'g' },
                &from[1..2]
            )
        } else {
            format!(
                "{from}{to}{}",
                promoted.and_then(promotion_letter).unwrap_or("")
            )
        };
        if !choices.iter().any(|(existing, _)| *existing == uci) {
            choices.push((uci, san));
        }
    }
    choices
}

/// Resolve spoken words against the actual playable position, including underpromotion and
/// castling. Unrelated speech never becomes a move by fuzzy matching.
fn spoken_move(text: &str, pos: &VariantPosition) -> Value {
    let mut words = tokens(text);
    if words.iter().any(|w| CONFIRM_WORDS.contains(&w.as_str()))
        && words
            .iter()
            .all(|w| CONFIRM_WORDS.contains(&w.as_str()) || CONFIRM_FILLER.contains(&w.as_str()))
    {
        return json!({ "kind": "confirm" });
    }
    if !words.is_empty() && words.iter().all(|w| CANCEL_WORDS.contains(&w.as_str())) {
        return json!({ "kind": "cancel" });
    }
    if matches!(words.first().map(String::as_str), Some("move" | "play")) {
        words.remove(0);
    }
    let castle = phrase(&words).replacen("castling", "castle", 1);
    let castle_side = match castle.as_str() {
        "castle kingside" | "castle king side" => Some("king"),
        "castle queenside" | "castle queen side" => Some("queen"),
        _ => None,
    };

    let mut promotion: Option<&'static str> = None;
    if let Some(last) = words.last().and_then(|w| role_word(w))
        && matches!(last, "queen" | "rook" | "bishop" | "knight")
    {
        promotion = Some(last);
        words.pop();
        if words.last().map(String::as_str) == Some("to") {
            words.pop();
        }
        if matches!(
            words.last().map(String::as_str),
            Some("promote" | "promotion" | "equals")
        ) {
            words.pop();
        }
    }
    let dest = words.pop();
    let capture = words.iter().any(|w| w == "takes" || w == "captures");
    let mut role: Option<&'static str> = None;
    if let Some(piece) = words.first().and_then(|w| role_word(w)) {
        role = Some(piece);
        words.remove(0);
    }
    if words.first().map(String::as_str) == Some("from") {
        words.remove(0);
    }
    // Recognizers can hear the separator “to” as “two”. Only reinterpret it between two complete
    // coordinates, where it cannot be a source rank.
    if words.len() == 2 && is_square(&words[0]) && words[1] == "2" {
        words.pop();
    }
    if matches!(
        words.last().map(String::as_str),
        Some("to" | "takes" | "captures")
    ) {
        words.pop();
    }
    let source = words.first().cloned();
    if castle_side.is_none()
        && (dest.as_deref().is_none_or(|d| !is_square(d))
            || words.len() > 1
            || source.as_deref().is_some_and(|s| !is_source(s)))
    {
        return json!({ "kind": "invalid" });
    }
    // A square alone means a pawn move; piece names and explicit origins are also supported.
    if castle_side.is_none() && role.is_none() && source.is_none() {
        role = Some("pawn");
    }
    let spoken = Spoken {
        castle_side,
        dest: dest.as_deref(),
        role,
        source: source.as_deref(),
        capture,
        promotion,
    };
    let mut choices = matching_moves(pos, spoken);
    // “Pawn to e5” is often heard as “pawn two e five”. A lone rank 2 that fits no move was that
    // “to”.
    if choices.is_empty() && source.as_deref() == Some("2") && role.is_some() {
        choices = matching_moves(
            pos,
            Spoken {
                source: None,
                ..spoken
            },
        );
    }
    if choices.is_empty() {
        return json!({ "kind": "invalid" });
    }
    let choices: Vec<Value> = choices
        .into_iter()
        .map(|(uci, san)| json!({ "uci": uci, "san": san }))
        .collect();
    json!({ "kind": "move", "choices": choices })
}

/* ── Board editor ── */

/// A JavaScript array index: an integer within `0..len` (anything else reads as undefined).
fn at_index(index: f64, len: usize) -> Option<usize> {
    (index.fract() == 0.0 && index >= 0.0 && (index as usize) < len).then_some(index as usize)
}

/// `Number(unit)` for one UTF-16 code unit of a square name (NaN when absent).
fn unit_number(unit: Option<&u16>) -> f64 {
    match unit {
        None => f64::NAN,
        Some(unit) => js::string_to_number(&String::from_utf16_lossy(&[*unit])),
    }
}

/// `charCodeAt(0) - 97` of a square name (NaN when empty).
fn file_offset(square: &str) -> Option<i64> {
    square
        .encode_utf16()
        .next()
        .map(|unit| i64::from(unit) - 97)
}

/// The piece letter on a square of a FEN board field (`e1` → `K`), or None.
fn piece_on(board: &str, square: &str) -> Option<char> {
    let units: Vec<u16> = square.encode_utf16().collect();
    let file = file_offset(square);
    let rows: Vec<&str> = board.split('/').collect();
    let row = rows[at_index(8.0 - unit_number(units.get(1)), rows.len())?];
    let mut index: i64 = 0;
    for c in row.chars() {
        if c.is_ascii_digit() {
            index += i64::from(c as u8 - b'0');
        } else {
            let hit = file == Some(index);
            index += 1;
            if hit {
                return Some(c);
            }
        }
        if file.is_some_and(|f| index > f) {
            return None;
        }
    }
    None
}

fn letter_for(role: &str) -> Option<char> {
    Some(match role {
        "king" => 'k',
        "queen" => 'q',
        "rook" => 'r',
        "bishop" => 'b',
        "knight" => 'n',
        "pawn" => 'p',
        _ => return None,
    })
}

/// Each digit of a board row stands for that many empty squares.
fn expand_digits(row: &str) -> String {
    let mut out = String::new();
    for c in row.chars() {
        if c.is_ascii_digit() {
            out.extend(std::iter::repeat_n('.', usize::from(c as u8 - b'0')));
        } else {
            out.push(c);
        }
    }
    out
}

/// Runs of empty squares back to digits.
fn compress_row(row: &str) -> String {
    let mut out = String::new();
    let mut run = 0usize;
    for c in row.chars() {
        if c == '.' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.push_str(&run.to_string());
            run = 0;
        }
        out.push(c);
    }
    if run > 0 {
        out.push_str(&run.to_string());
    }
    out
}

/// The board field with `square` holding `piece`, or emptied when `piece` is None.
fn with_piece(board: &str, square: &str, piece: Option<(&str, &str)>) -> Result<String> {
    let mut rows: Vec<String> = board.split('/').map(expand_digits).collect();
    let units: Vec<u16> = square.encode_utf16().collect();
    let rank =
        at_index(8.0 - unit_number(units.get(1)), rows.len()).ok_or("the board has no such row")?;
    let letter = match piece {
        None => '.',
        Some((color, role)) => {
            let letter = letter_for(role).ok_or("unknown piece role")?;
            if color == "white" {
                letter.to_ascii_uppercase()
            } else {
                letter
            }
        }
    };
    let mut cells: Vec<Option<char>> = rows[rank].chars().map(Some).collect();
    // A file past the board's end leaves holes, which `join` writes as nothing.
    if let Some(file) = file_offset(square).filter(|file| *file >= 0) {
        let file = file as usize;
        if file >= cells.len() {
            cells.resize(file + 1, None);
        }
        cells[file] = Some(letter);
    }
    rows[rank] = cells.iter().flatten().collect();
    Ok(rows
        .iter()
        .map(|row| compress_row(row))
        .collect::<Vec<_>>()
        .join("/"))
}

fn with_piece_call(args: &[Value]) -> Result<Value> {
    let board = text(args, 0)?;
    let square = text(args, 1)?;
    let piece = match args.get(2).filter(|v| !v.is_null()) {
        None => None,
        Some(piece) => Some((
            piece
                .get("color")
                .and_then(Value::as_str)
                .ok_or("piece.color must be a string")?,
            piece
                .get("role")
                .and_then(Value::as_str)
                .ok_or("piece.role must be a string")?,
        )),
    };
    with_piece(board, square, piece).map(Value::String)
}

/// The castling rights `[K, Q, k, q]` the king and rook squares still allow.
fn castling_available(board: &str) -> [bool; 4] {
    let at = |square: &str, piece: char| piece_on(board, square) == Some(piece);
    [
        at("e1", 'K') && at("h1", 'R'),
        at("e1", 'K') && at("a1", 'R'),
        at("e8", 'k') && at("h8", 'r'),
        at("e8", 'k') && at("a8", 'r'),
    ]
}

fn castling_available_json(board: &str) -> Value {
    let [k_upper, q_upper, k_lower, q_lower] = castling_available(board);
    json!({ "K": k_upper, "Q": q_upper, "k": k_lower, "q": q_lower })
}

/// Squares a pawn could just have skipped with a double step, so the side to move might capture
/// en passant there.
fn en_passant_squares(board: &str, turn: &str) -> Vec<String> {
    let (pawn, rank, passed, start) = if turn == "white" {
        ('p', '5', '6', '7')
    } else {
        ('P', '4', '3', '2')
    };
    FILES
        .iter()
        .filter(|file| {
            piece_on(board, &format!("{file}{rank}")) == Some(pawn)
                && piece_on(board, &format!("{file}{passed}")).is_none()
                && piece_on(board, &format!("{file}{start}")).is_none()
        })
        .map(|file| format!("{file}{passed}"))
        .collect()
}

/// The FEN of an editor setup: castling rights and en passant only where the pieces allow them.
fn setup_fen(setup: &Value) -> Result<String> {
    let board = setup
        .get("board")
        .and_then(Value::as_str)
        .ok_or("setup.board must be a string")?;
    let turn = setup
        .get("turn")
        .and_then(Value::as_str)
        .ok_or("setup.turn must be a string")?;
    let ep = setup.get("ep").and_then(Value::as_str).unwrap_or("");
    let ticked = |right: &str| {
        setup
            .get("castling")
            .and_then(|castling| castling.get(right))
            .and_then(Value::as_bool)
            == Some(true)
    };
    let available = castling_available(board);
    let rights: String = ["K", "Q", "k", "q"]
        .iter()
        .zip(available)
        .filter(|(right, allowed)| *allowed && ticked(right))
        .map(|(right, _)| *right)
        .collect();
    let placement = if turn == "white" { "w" } else { "b" };
    let ep = if en_passant_squares(board, turn).iter().any(|s| s == ep) {
        ep
    } else {
        "-"
    };
    Ok(format!(
        "{board} {placement} {} {ep} 0 1",
        if rights.is_empty() { "-" } else { &rights }
    ))
}

fn setup_fen_call(args: &[Value]) -> Result<Value> {
    let setup = args
        .first()
        .filter(|v| v.is_object())
        .ok_or("setup must be an object")?;
    setup_fen(setup).map(Value::String)
}

/* ── Coordinates ── */

/// The index into the 64 squares of a draw in [0, 1), as `Math.floor(random() * 64)`.
fn square_index(draw: f64) -> Option<usize> {
    let index = (draw * 64.0).floor();
    (0.0..64.0).contains(&index).then_some(index as usize)
}

/// The first draw whose square is not `previous`, as `randomSquare` retries; null when the
/// draws run out.
fn random_square(previous: Option<&str>, draws: &[f64]) -> Value {
    let squares = all_squares();
    for draw in draws {
        let Some(index) = square_index(*draw) else {
            continue;
        };
        let square = &squares[index];
        if Some(square.as_str()) != previous {
            return json!(square);
        }
    }
    Value::Null
}

fn random_square_call(args: &[Value]) -> Result<Value> {
    let previous = args.first().and_then(Value::as_str);
    Ok(random_square(previous, &numbers(args, 1)?))
}

fn square_color(square: &str) -> &'static str {
    let (file, rank) = knight_index(square);
    if (file + rank) % 2.0 == 0.0 {
        "dark"
    } else {
        "light"
    }
}

/* ── Knight paths ── */

const JUMPS: [(f64, f64); 8] = [
    (1.0, 2.0),
    (2.0, 1.0),
    (2.0, -1.0),
    (1.0, -2.0),
    (-1.0, -2.0),
    (-2.0, -1.0),
    (-2.0, 1.0),
    (-1.0, 2.0),
];

/// The file and rank offsets of a square (NaN for anything not a square).
fn knight_index(square: &str) -> (f64, f64) {
    let units: Vec<u16> = square.encode_utf16().collect();
    let file = file_offset(square).map_or(f64::NAN, |f| f as f64);
    (file, unit_number(units.get(1)) - 1.0)
}

fn knight_moves(square: &str) -> Vec<String> {
    let (file, rank) = knight_index(square);
    JUMPS
        .iter()
        .filter_map(|(df, dr)| {
            let f = file + df;
            let r = rank + dr;
            ((0.0..8.0).contains(&f) && (0.0..8.0).contains(&r))
                .then(|| format!("{}{}", FILES[f as usize], r as usize + 1))
        })
        .collect()
}

/// Fewest jumps from `from` to `to`; None when no path exists.
fn knight_distance(from: &str, to: &str) -> Option<u32> {
    if from == to {
        return Some(0);
    }
    let mut seen: HashSet<String> = HashSet::from([from.to_string()]);
    let mut frontier = vec![from.to_string()];
    let mut steps = 1;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for square in &frontier {
            for target in knight_moves(square) {
                if target == to {
                    return Some(steps);
                }
                if seen.insert(target.clone()) {
                    next.push(target);
                }
            }
        }
        frontier = next;
        steps += 1;
    }
    None
}

/// The first pair of draws whose squares are `min`–`max` jumps apart, as `knightChallenge`
/// retries; null when the draws run out.
fn knight_challenge(min: f64, max: f64, draws: &[f64]) -> Value {
    let squares = all_squares();
    for pair in draws.chunks_exact(2) {
        let (Some(a), Some(b)) = (square_index(pair[0]), square_index(pair[1])) else {
            continue;
        };
        let (from, to) = (&squares[a], &squares[b]);
        let best = knight_distance(from, to);
        let best_value = best.map_or(f64::INFINITY, f64::from);
        if best_value >= min && best_value <= max {
            return json!({
                "from": from,
                "to": to,
                "best": best.map_or(Value::Null, |n| json!(n)),
            });
        }
    }
    Value::Null
}

fn knight_challenge_call(args: &[Value]) -> Result<Value> {
    Ok(knight_challenge(
        number(args, 0)?,
        number(args, 1)?,
        &numbers(args, 2)?,
    ))
}

/// `${n || ''}`: zero and NaN print as nothing.
fn js_or_empty(n: f64) -> String {
    if n == 0.0 || n.is_nan() {
        String::new()
    } else {
        js::number_to_string(n)
    }
}

/// A board with one white knight, as a FEN (the board only draws it).
fn knight_fen(square: &str) -> String {
    let (file, rank) = knight_index(square);
    let rows: Vec<String> = (0..8)
        .map(|r| {
            let row = 7 - r;
            if f64::from(row) == rank {
                format!("{}N{}", js_or_empty(file), js_or_empty(7.0 - file))
            } else {
                "8".into()
            }
        })
        .collect();
    format!("{} w - - 0 1", rows.join("/"))
}

/* ── Material ── */

/// Material balance of a FEN's placement in pawns: positive when White is ahead.
fn material_balance(fen: &str) -> i64 {
    let placement = fen.split(' ').next().unwrap_or("");
    let mut balance = 0;
    for c in placement.chars() {
        let lower: String = c.to_lowercase().collect();
        let value = match lower.as_str() {
            "p" => 1,
            "n" | "b" => 3,
            "r" => 5,
            "q" => 9,
            _ => continue,
        };
        if c.to_uppercase().collect::<String>() == c.to_string() {
            balance += value;
        } else {
            balance -= value;
        }
    }
    balance
}
