//! Spoken commands and moves.
//!
//! JavaScript semantics are mirrored deliberately: `toLowerCase`, `\s` splitting, `trim`,
//! `Number(…)` coercion and `String(…)` formatting (see `crate::js`). No regex crate: each
//! pattern below is the hand-written equivalent of the regex it names.

use serde::Serialize;
use shakmaty::variant::VariantPosition;
use shakmaty::{Color, Position};
use std::sync::OnceLock;

use crate::js;
use crate::position::{self, LegalMove};
use crate::squares::FILES;

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
crate::union!(
    /// A game command said aloud.
    GameCommand {
        Takeback = "takeback",
        NewGame = "newGame",
        Resign = "resign",
        Flip = "flip",
    }
);
crate::union!(
    /// An analysis board command said aloud.
    AnalysisCommand {
        Back = "back",
        Forward = "forward",
        Start = "start",
        End = "end",
        Best = "best",
        Flip = "flip",
        Engine = "engine",
    }
);
const GAME_COMMANDS: [(&str, GameCommand); 6] = [
    ("take back", GameCommand::Takeback),
    ("undo", GameCommand::Takeback),
    ("new game", GameCommand::NewGame),
    ("resign", GameCommand::Resign),
    ("flip board", GameCommand::Flip),
    ("switch sides", GameCommand::Flip),
];
const ANALYSIS_COMMANDS: [(&str, AnalysisCommand); 18] = [
    ("back", AnalysisCommand::Back),
    ("go back", AnalysisCommand::Back),
    ("previous", AnalysisCommand::Back),
    ("undo", AnalysisCommand::Back),
    ("take back", AnalysisCommand::Back),
    ("forward", AnalysisCommand::Forward),
    ("next", AnalysisCommand::Forward),
    ("go forward", AnalysisCommand::Forward),
    ("first move", AnalysisCommand::Start),
    ("go to start", AnalysisCommand::Start),
    ("last move", AnalysisCommand::End),
    ("go to end", AnalysisCommand::End),
    ("best move", AnalysisCommand::Best),
    ("play best move", AnalysisCommand::Best),
    ("flip board", AnalysisCommand::Flip),
    ("toggle engine", AnalysisCommand::Engine),
    ("engine on", AnalysisCommand::Engine),
    ("engine off", AnalysisCommand::Engine),
];
crate::union!(
    /// A training board's command said aloud: it does what the page's button of that name does.
    PuzzleCommand {
        /// The next puzzle, position or step.
        Next = "next",
        /// This position again from its start.
        Retry = "retry",
        /// Show (or hide) the hint.
        Hint = "hint",
        /// The previous position.
        Back = "back",
        /// Show the move to play (the repertoire trainer's Show move).
        ShowMove = "showMove",
        /// A line from its start again (the repertoire trainer's Restart line).
        RestartLine = "restartLine",
    }
);
/// What a training board takes besides moves; each page listens for those its buttons offer.
const PUZZLE_COMMANDS: [(&str, PuzzleCommand); 12] = [
    ("next", PuzzleCommand::Next),
    ("next puzzle", PuzzleCommand::Next),
    ("retry", PuzzleCommand::Retry),
    ("try again", PuzzleCommand::Retry),
    ("restart", PuzzleCommand::Retry),
    ("hint", PuzzleCommand::Hint),
    ("show hint", PuzzleCommand::Hint),
    ("back", PuzzleCommand::Back),
    ("go back", PuzzleCommand::Back),
    ("previous", PuzzleCommand::Back),
    ("show move", PuzzleCommand::ShowMove),
    ("restart line", PuzzleCommand::RestartLine),
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
    /// Moves and every training board command (`commands_grammar` for a page's own).
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
    commands_grammar(PuzzleCommand::ALL)
}

/// Moves and the phrases of `commands`: what a training board listens for, given the commands
/// its page has buttons for.
pub fn commands_grammar(commands: &[PuzzleCommand]) -> Vec<String> {
    let mut out = move_grammar();
    out.extend(
        PUZZLE_COMMANDS
            .iter()
            .filter(|(_, command)| commands.contains(command))
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
    let mut rest = &words[..];
    while let [word, tail @ ..] = rest {
        let word = word.as_str();
        let next = tail.first().map(String::as_str);
        let after = tail.get(1).map(String::as_str);
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
                rest = tail.get(1..).unwrap_or_default();
            }
            (file, _) => {
                out.push(
                    file.map(String::from)
                        .or_else(|| rank_word(word).map(String::from))
                        .unwrap_or_else(|| word.to_string()),
                );
                rest = tail;
            }
        }
    }
    out
}

/* ── Commands and choices ── */

/// A game command said aloud.
pub fn game_command(text: &str) -> Option<GameCommand> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    GAME_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

/// An analysis board command said aloud.
pub fn analysis_command(text: &str) -> Option<AnalysisCommand> {
    let mut key = phrase(&split_words(text));
    if key == "takeback" {
        key = "take back".into();
    }
    ANALYSIS_COMMANDS
        .iter()
        .find(|(phrase, _)| *phrase == key)
        .map(|(_, command)| *command)
}

/// A training board command said aloud: next, retry, hint or back.
pub fn puzzle_command(text: &str) -> Option<PuzzleCommand> {
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
        use super::{PuzzleCommand, puzzle_command};
        assert_eq!(puzzle_command("next"), Some(PuzzleCommand::Next));
        assert_eq!(puzzle_command("  Next Puzzle "), Some(PuzzleCommand::Next));
        assert_eq!(puzzle_command("e four"), None);
        assert_eq!(puzzle_command("next move"), None);
    }

    /// Retry, hint and back, each in the words a player would use for its button.
    #[test]
    fn retry_hint_and_back_are_said_as_their_buttons_read() {
        use super::{PuzzleCommand, puzzle_command};
        for (said, command) in [
            ("retry", PuzzleCommand::Retry),
            ("Try again!", PuzzleCommand::Retry),
            ("restart", PuzzleCommand::Retry),
            ("hint", PuzzleCommand::Hint),
            ("show hint", PuzzleCommand::Hint),
            ("back", PuzzleCommand::Back),
            ("go back", PuzzleCommand::Back),
            ("previous", PuzzleCommand::Back),
            ("Show move", PuzzleCommand::ShowMove),
            ("restart line", PuzzleCommand::RestartLine),
        ] {
            assert_eq!(puzzle_command(said), Some(command), "{said}");
        }
        // Moves and other pages' commands are not these.
        assert_eq!(puzzle_command("take back"), None);
        assert_eq!(puzzle_command("hint please"), None);
        assert_eq!(puzzle_command("knight f three"), None);
    }

    /// A page listens only for the commands it has buttons for.
    #[test]
    fn a_page_listens_for_its_own_commands() {
        use super::{PuzzleCommand, commands_grammar};
        let has = |words: &[String], word: &str| words.iter().any(|w| w == word);
        let drills = commands_grammar(&[PuzzleCommand::Hint, PuzzleCommand::Retry]);
        assert!(has(&drills, "hint") && has(&drills, "try again") && has(&drills, "e four"));
        assert!(!has(&drills, "next") && !has(&drills, "back"));
        // The repertoire trainer's own buttons are heard only where it offers them.
        assert!(!has(&drills, "show move") && !has(&drills, "restart line"));
        let trainer = commands_grammar(&[PuzzleCommand::ShowMove, PuzzleCommand::RestartLine]);
        assert!(has(&trainer, "show move") && has(&trainer, "restart line"));
        let all = grammar(Grammar::Puzzle);
        for word in ["next", "next puzzle", "retry", "hint", "back", "previous"] {
            assert!(has(all, word), "{word}");
        }
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

    fn say(text: &str, fen: &str) -> super::SpokenMove {
        super::spoken_move(text, &crate::rules::standard_from_fen(fen).unwrap())
    }

    /// The `(uci, san)` of every choice a phrase offers at `fen`.
    fn choices(text: &str, fen: &str) -> Vec<(String, String)> {
        match say(text, fen) {
            super::SpokenMove::Move { choices } => {
                choices.into_iter().map(|c| (c.uci, c.san)).collect()
            }
            other => panic!("{text:?} was read as {other:?}"),
        }
    }

    fn ucis(text: &str, fen: &str) -> Vec<String> {
        choices(text, fen).into_iter().map(|(uci, _)| uci).collect()
    }

    const BOTH_CASTLES: &str = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    const PROMOTION: &str = "8/4P3/8/8/8/8/k7/4K3 w - - 0 1";

    #[test]
    fn a_square_is_a_file_then_a_rank_and_nothing_else() {
        use super::is_square;
        assert!(is_square("a1") && is_square("h8"));
        for not in ["", "a", "a1x", "i1", "a9", "1a", "abc"] {
            assert!(!is_square(not), "{not:?}");
        }
    }

    #[test]
    fn a_source_is_a_square_a_file_or_a_rank() {
        use super::is_source;
        for yes in ["e4", "e", "4", "a", "h", "1", "8"] {
            assert!(is_source(yes), "{yes:?}");
        }
        for not in ["", "ab", "i", "9", "0", "e9", "zz", "e4x", "xyz"] {
            assert!(!is_source(not), "{not:?}");
        }
    }

    #[test]
    fn an_eight_before_a_rank_is_file_a_unless_a_square_follows() {
        use super::tokens;
        assert_eq!(tokens("eight three"), ["a3"]);
        assert_eq!(tokens("eight three alpha"), ["8", "3", "a"]);
        assert_eq!(tokens("rook eight to a three"), ["rook", "8", "to", "a3"]);
        assert_eq!(tokens("knight eight three"), ["knight", "a3"]);
    }

    #[test]
    fn only_confirm_words_and_filler_confirm() {
        use super::SpokenMove::Confirm;
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(say("yes", start), Confirm);
        assert_eq!(say("confirm move", start), Confirm);
        assert_eq!(say("yeah play", start), Confirm);
        assert_ne!(say("play", start), Confirm);
        assert_ne!(say("yes e four", start), Confirm);
    }

    #[test]
    fn castling_is_said_as_a_side() {
        for (text, uci, san) in [
            ("castle kingside", "e1g1", "O-O"),
            ("castle king side", "e1g1", "O-O"),
            ("castling kingside", "e1g1", "O-O"),
            ("castle queenside", "e1c1", "O-O-O"),
            ("castle queen side", "e1c1", "O-O-O"),
            ("play castle queenside", "e1c1", "O-O-O"),
        ] {
            assert_eq!(
                choices(text, BOTH_CASTLES),
                [(uci.to_string(), san.to_string())],
                "{text}"
            );
        }
    }

    #[test]
    fn castling_with_check_is_still_castling() {
        // The rook lands on f1 and checks the king on f8.
        assert_eq!(
            choices("castle kingside", "5k2/8/8/8/8/8/8/4K2R w K - 0 1"),
            [("e1g1".to_string(), "O-O+".to_string())]
        );
    }

    #[test]
    fn a_chess960_castle_stays_king_takes_rook() {
        assert_eq!(
            ucis("castle kingside", "1k6/8/8/8/8/8/8/1K5R w H - 0 1"),
            ["b1h1"]
        );
    }

    #[test]
    fn a_rook_move_is_not_castling() {
        assert_eq!(
            ucis("rook to h one", "k7/8/8/8/8/8/8/K3R3 w - - 0 1"),
            ["e1h1"]
        );
    }

    #[test]
    fn a_promotion_offers_only_the_named_piece() {
        for phrase in ["e eight queen", "e eight to queen", "e eight promote queen"] {
            assert_eq!(ucis(phrase, PROMOTION), ["e7e8q"], "{phrase}");
        }
        assert_eq!(ucis("e eight knight", PROMOTION), ["e7e8n"]);
    }

    #[test]
    fn more_than_one_word_before_the_destination_is_not_a_move() {
        // Nb1 could reach c3, but "b one a" is two words of origin.
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(say("b one a c three", start), super::SpokenMove::Invalid);
        assert_eq!(ucis("b one c three", start), ["b1c3"]);
    }
}
