//! Spoken commands and moves, coordinates, knight paths and material.
//!
//! JavaScript semantics are mirrored deliberately: `toLowerCase`, `\s` splitting, `trim`,
//! `Number(…)` coercion and `String(…)` formatting (see `crate::js`). No regex crate: each
//! pattern below is the hand-written equivalent of the regex it names.

use serde::Serialize;
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position};
use std::collections::HashSet;
use std::sync::OnceLock;

use crate::editor::{file_offset, unit_number};
use crate::js;
use crate::position::{self, LegalMove};

pub(crate) const FILES: [char; 8] = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
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
/// What a finished puzzle takes besides moves.
const PUZZLE_COMMANDS: [(&str, &str); 2] = [("next", "next"), ("next puzzle", "next")];
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

/* ── Grammar lists ── */

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// The voice grammars: the words each kind of page listens for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grammar {
    /// Squares alone, for the coordinate and knight drills.
    Coordinate,
    /// Moves, confirmations and cancellations.
    Move,
    /// Moves and the game's commands.
    Game,
    /// The board editor's pieces, squares and phrases.
    Editor,
    /// Moves and the analysis board's commands.
    Analysis,
    /// Moves and going on to the next puzzle.
    Puzzle,
}

/// The words a grammar listens for, built once.
pub fn grammar(grammar: Grammar) -> &'static [String] {
    static LISTS: [OnceLock<Vec<String>>; 6] = [const { OnceLock::new() }; 6];
    let (index, build): (usize, fn() -> Vec<String>) = match grammar {
        Grammar::Coordinate => (0, coordinate_grammar),
        Grammar::Move => (1, move_grammar),
        Grammar::Game => (2, game_grammar),
        Grammar::Editor => (3, editor_grammar),
        Grammar::Analysis => (4, analysis_grammar),
        Grammar::Puzzle => (5, puzzle_grammar),
    };
    LISTS[index].get_or_init(build)
}

/// Every square, file by file: a1, a2, … h8.
pub fn all_squares() -> Vec<String> {
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

fn puzzle_grammar() -> Vec<String> {
    let mut out = move_grammar();
    out.extend(PUZZLE_COMMANDS.iter().map(|(phrase, _)| phrase.to_string()));
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

/// A game command said aloud: `takeback`, `newGame`, `resign` or `flip`.
pub fn game_command(text: &str) -> Option<&'static str> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    GAME_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

/// An analysis board command said aloud: `back`, `forward`, `start`, `end`, `best`, `flip` or
/// `engine`.
pub fn analysis_command(text: &str) -> Option<&'static str> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    ANALYSIS_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

/// A puzzle command said aloud: `next`.
pub fn puzzle_command(text: &str) -> Option<&'static str> {
    let key = phrase(&split_words(text));
    PUZZLE_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

/// A choice's number said aloud, 1 to 8.
pub fn spoken_choice(text: &str) -> Option<u8> {
    let normalized = js::trim(text).to_lowercase();
    let rank = rank_word(&normalized).map_or(normalized.clone(), String::from);
    is_rank_digit(&rank).then(|| rank.as_bytes()[0] - b'0')
}

/// One square said aloud (“echo four”).
pub fn spoken_square(text: &str) -> Option<String> {
    let words = tokens(text);
    match words.as_slice() {
        [only] if is_square(only) => Some(only.clone()),
        _ => None,
    }
}

/// What a phrase said to the board editor asks for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EditorAction {
    Clear,
    Start,
    Turn {
        color: String,
    },
    Flip,
    Analyze,
    Remove {
        square: String,
    },
    Place {
        square: String,
        color: String,
        role: &'static str,
    },
}

/// The board editor's whole-phrase actions.
fn editor_phrase(key: &str) -> Option<EditorAction> {
    let turn = |color: &str| EditorAction::Turn {
        color: color.to_string(),
    };
    Some(match key {
        "clear board" | "empty board" | "clear the board" => EditorAction::Clear,
        "starting position" | "start position" | "reset board" | "reset the board" => {
            EditorAction::Start
        }
        "white to move" | "white to play" => turn("white"),
        "black to move" | "black to play" => turn("black"),
        "flip board" | "flip the board" => EditorAction::Flip,
        "analyze" | "analysis" => EditorAction::Analyze,
        _ => return None,
    })
}

/// “White knight F three”, “black king on E eight”, “remove E four”, “clear board”, … A piece
/// without a colour takes `color`, the last one used.
pub fn spoken_edit(text: &str, color: &str) -> Option<EditorAction> {
    if let Some(fixed) = editor_phrase(&phrase(&split_words(text))) {
        return Some(fixed);
    }
    let mut words: Vec<String> = tokens(text)
        .into_iter()
        .filter(|w| !matches!(w.as_str(), "on" | "to" | "at" | "the"))
        .collect();
    let square = words.pop()?;
    if !is_square(&square) {
        return None;
    }
    let mut rest = words;
    if rest.len() == 1 && REMOVE_WORDS.contains(&rest[0].as_str()) {
        return Some(EditorAction::Remove { square });
    }
    let mut spoken_color: Option<String> = None;
    if matches!(rest.first().map(String::as_str), Some("white" | "black")) {
        spoken_color = Some(rest.remove(0));
    }
    match rest.as_slice() {
        [piece] => Some(EditorAction::Place {
            square,
            color: spoken_color.unwrap_or_else(|| color.to_string()),
            role: role_word(piece)?,
        }),
        _ => None,
    }
}

/* ── Moves ── */

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
        // Only the standard geometry has that spelling: in Chess960 a king two squares away
        // can be an ordinary king move (b1c1), so castling stays king-takes-rook there.
        let standard_castle = from.starts_with('e') && (to.starts_with('a') || to.starts_with('h'));
        let uci = if san.starts_with("O-O") && standard_castle {
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

/// What a phrase said at a board means.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpokenMove {
    Confirm,
    Cancel,
    Invalid,
    /// Every legal move the words could be, in the rules' move order.
    Move {
        choices: Vec<MoveChoice>,
    },
}

/// One legal move a phrase could mean; castling in standard UCI (e1g1) outside Chess960.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MoveChoice {
    pub uci: String,
    pub san: String,
}

/// Resolve spoken words against the actual playable position, including underpromotion and
/// castling. Unrelated speech never becomes a move by fuzzy matching.
pub fn spoken_move(text: &str, pos: &VariantPosition) -> SpokenMove {
    let mut words = tokens(text);
    if words.iter().any(|w| CONFIRM_WORDS.contains(&w.as_str()))
        && words
            .iter()
            .all(|w| CONFIRM_WORDS.contains(&w.as_str()) || CONFIRM_FILLER.contains(&w.as_str()))
    {
        return SpokenMove::Confirm;
    }
    if !words.is_empty() && words.iter().all(|w| CANCEL_WORDS.contains(&w.as_str())) {
        return SpokenMove::Cancel;
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
        return SpokenMove::Invalid;
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
        return SpokenMove::Invalid;
    }
    SpokenMove::Move {
        choices: choices
            .into_iter()
            .map(|(uci, san)| MoveChoice { uci, san })
            .collect(),
    }
}

/* ── Coordinates ── */

/// The index into the 64 squares of a draw in [0, 1), as `Math.floor(random() * 64)`.
fn square_index(draw: f64) -> Option<usize> {
    let index = (draw * 64.0).floor();
    (0.0..64.0).contains(&index).then_some(index as usize)
}

/// The first draw whose square is not `previous`; none when the draws run out. With `allowed`, draws pick among those squares only (in the given order); a
/// single allowed square is returned even when it is `previous`, since there is no other.
pub fn random_square(
    previous: Option<&str>,
    draws: &[f64],
    allowed: Option<&[String]>,
) -> Option<String> {
    let squares = match allowed {
        Some(allowed) if !allowed.is_empty() => allowed.to_vec(),
        _ => all_squares(),
    };
    if squares.len() == 1 {
        return squares.into_iter().next();
    }
    for draw in draws {
        let index = (draw * squares.len() as f64).floor();
        if !(0.0..squares.len() as f64).contains(&index) {
            continue;
        }
        let square = &squares[index as usize];
        if Some(square.as_str()) != previous {
            return Some(square.clone());
        }
    }
    None
}

/// Whether a square is light or dark.
pub fn square_color(square: &str) -> &'static str {
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

/// The squares a knight on `square` reaches in one jump.
pub fn knight_moves(square: &str) -> Vec<String> {
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
pub fn knight_distance(from: &str, to: &str) -> Option<u32> {
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

/// A knight's journey: from one square to another, `best` jumps apart (none when unreachable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KnightChallenge {
    pub from: String,
    pub to: String,
    pub best: Option<u32>,
}

/// The first pair of draws whose squares are `min`–`max` jumps apart; none when the draws run
/// out.
pub fn knight_challenge(min: f64, max: f64, draws: &[f64]) -> Option<KnightChallenge> {
    let squares = all_squares();
    for pair in draws.as_chunks::<2>().0 {
        let (Some(a), Some(b)) = (square_index(pair[0]), square_index(pair[1])) else {
            continue;
        };
        let (from, to) = (&squares[a], &squares[b]);
        let best = knight_distance(from, to);
        let best_value = best.map_or(f64::INFINITY, f64::from);
        if best_value >= min && best_value <= max {
            return Some(KnightChallenge {
                from: from.clone(),
                to: to.clone(),
                best,
            });
        }
    }
    None
}

/// A number's text; zero and NaN print as nothing.
fn js_or_empty(n: f64) -> String {
    if n == 0.0 || n.is_nan() {
        String::new()
    } else {
        js::number_to_string(n)
    }
}

/// A board with one white knight, as a FEN (the board only draws it).
pub fn knight_fen(square: &str) -> String {
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

fn balance_of(white: &[u32; 6], black: &[u32; 6]) -> i64 {
    let value = [1, 3, 3, 5, 9, 0];
    (0..6)
        .map(|i| (i64::from(white[i]) - i64::from(black[i])) * value[i])
        .sum()
}

/// One pass over the placement: pieces per side, indexed pawn, knight, bishop, rook, queen, king.
fn material_counts(fen: &str) -> ([u32; 6], [u32; 6]) {
    let placement = fen.split(' ').next().unwrap_or("");
    let mut white = [0u32; 6];
    let mut black = [0u32; 6];
    for byte in placement.bytes() {
        let (counts, piece) = if byte.is_ascii_uppercase() {
            (&mut white, byte.to_ascii_lowercase())
        } else {
            (&mut black, byte)
        };
        let index = match piece {
            b'p' => 0,
            b'n' => 1,
            b'b' => 2,
            b'r' => 3,
            b'q' => 4,
            b'k' => 5,
            _ => continue,
        };
        counts[index] += 1;
    }
    (white, black)
}

/// Roles from most to least valuable, as Lichess lists a material difference.
const MATERIAL_ROLES: [(char, &str); 6] = [
    ('k', "king"),
    ('q', "queen"),
    ('r', "rook"),
    ('b', "bishop"),
    ('n', "knight"),
    ('p', "pawn"),
];

/// Lichess's material difference: the pieces each side has beyond the other's, most valuable
/// first, and `score`, the balance in pawns (positive when White is ahead).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaterialDiff {
    pub white: Vec<&'static str>,
    pub black: Vec<&'static str>,
    pub score: i64,
}

pub fn material_diff(fen: &str) -> MaterialDiff {
    let (white_counts, black_counts) = material_counts(fen);
    let mut white = Vec::new();
    let mut black = Vec::new();
    for (letter, role) in MATERIAL_ROLES {
        let index = match letter {
            'k' => 5,
            'q' => 4,
            'r' => 3,
            'b' => 2,
            'n' => 1,
            _ => 0,
        };
        let ours = white_counts[index];
        let theirs = black_counts[index];
        let (side, extra) = if ours > theirs {
            (&mut white, ours - theirs)
        } else {
            (&mut black, theirs - ours)
        };
        side.extend(std::iter::repeat_n(role, extra as usize));
    }
    MaterialDiff {
        white,
        black,
        score: balance_of(&white_counts, &black_counts),
    }
}

#[cfg(test)]
mod tests {
    use super::{Grammar, grammar};

    #[test]
    fn the_grammars_hold_their_words() {
        let moves = grammar(Grammar::Move);
        assert!(moves.contains(&"e four".to_string()) && moves.contains(&"knight".to_string()));
        assert!(grammar(Grammar::Game).contains(&"take back".to_string()));
        let puzzle = grammar(Grammar::Puzzle);
        assert!(
            puzzle.contains(&"next puzzle".to_string()) && puzzle.contains(&"e four".to_string())
        );
        assert!(!moves.contains(&"next".to_string()));
    }

    #[test]
    fn next_goes_on_to_the_next_puzzle() {
        use super::puzzle_command;
        assert_eq!(puzzle_command("next"), Some("next"));
        assert_eq!(puzzle_command("  Next Puzzle "), Some("next"));
        assert_eq!(puzzle_command("e four"), None);
        assert_eq!(puzzle_command("next move"), None);
    }

    /// Silence is no move and no cancel; a cancel word is a cancel, and a square a move.
    #[test]
    fn silence_is_not_a_cancel() {
        use super::{SpokenMove, spoken_move};
        let start = crate::rules::default_position(shakmaty::variant::Variant::Chess);
        assert_eq!(spoken_move("", &start), SpokenMove::Invalid);
        assert_eq!(spoken_move("cancel", &start), SpokenMove::Cancel);
        assert!(matches!(
            spoken_move("e four", &start),
            SpokenMove::Move { choices } if choices[0].uci == "e2e4"
        ));
    }

    /// "Pawn to e5" heard as "pawn two e five": no Black pawn leaves the second rank, so the
    /// "two" was the "to".
    #[test]
    fn a_lone_two_that_fits_no_move_was_a_to() {
        use super::{SpokenMove, spoken_move};
        let after_e4 = crate::rules::standard_from_fen(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
        )
        .unwrap();
        assert!(matches!(
            spoken_move("pawn two e five", &after_e4),
            SpokenMove::Move { choices } if choices.len() == 1 && choices[0].uci == "e7e5"
        ));
    }
}
