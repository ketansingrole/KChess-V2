//! Opening names (`crates/kchess-wasm/js/openings.ts`): Lichess's CC0 named lines, as the table
//! `tooling/make-openings.mjs` writes. Each position takes the name of the shortest line reaching it
//! (the first on a tie, like Lichess), and the names form the family tree the openings page browses.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::Position;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::{Out, js_slice_end, to_value, value_arg};
use crate::{js, position, rules};

const OPENINGS_JSON: &str = include_str!("../../data/openings.json");
const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// `OpeningName`
#[derive(Debug, Serialize)]
pub struct OpeningName {
    pub eco: String,
    pub name: String,
}

/// One named line: its ECO code, its name and its UCI moves from the starting position.
struct Line {
    eco: String,
    name: String,
    moves: Vec<String>,
}

#[derive(Deserialize, Default)]
struct Data {
    lines: Vec<(String, String, String)>,
}

fn lines() -> &'static [Line] {
    static LINES: OnceLock<Vec<Line>> = OnceLock::new();
    LINES.get_or_init(|| {
        let data: Data = serde_json::from_str(OPENINGS_JSON).unwrap_or_default();
        data.lines
            .into_iter()
            .map(|(eco, name, moves)| Line {
                eco,
                name,
                moves: moves.split(' ').map(str::to_string).collect(),
            })
            .collect()
    })
}

/// The named positions: the index of the line naming each, keyed by the first four FEN fields.
fn table() -> &'static HashMap<String, usize> {
    static TABLE: OnceLock<HashMap<String, usize>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: HashMap<String, usize> = HashMap::new();
        let Some(start) = rules::setup_start("standard", START) else {
            return table;
        };
        let all = lines();
        for (index, line) in all.iter().enumerate() {
            let mut pos = start.clone();
            let played = line.moves.iter().all(|uci| {
                let Some(m) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
                    return false;
                };
                pos.play_unchecked(m);
                true
            });
            if !played {
                continue;
            }
            let key = repetition_key(&rules::make_fen(&pos)).to_string();
            let shorter = table
                .get(&key)
                .is_none_or(|&known| line.moves.len() < all[known].moves.len());
            if shorter {
                table.insert(key, index);
            }
        }
        table
    })
}

/// `repetitionKey`: the first four FEN fields, borrowed from the FEN.
fn repetition_key(fen: &str) -> &str {
    let mut seen = 0;
    for (index, byte) in fen.bytes().enumerate() {
        if byte == b' ' {
            seen += 1;
            if seen == 4 {
                return &fen[..index];
            }
        }
    }
    fen
}

/// `openingAt`: the name of the opening reached after `ply` moves. As Lichess suggests, step back
/// from the position until a named one is found. Only games under standard rules have names.
pub fn opening_at<S: AsRef<str>>(
    variant: &str,
    fen: &str,
    moves: &[S],
    ply: f64,
) -> Option<OpeningName> {
    if variant != "standard" {
        return None;
    }
    let mut pos = rules::setup_start("standard", fen)?;
    let books = table();
    let named = lines();
    let end = js_slice_end(moves.len(), ply);
    // Walk forward once, keeping the last named position: the old code built every FEN and
    // repetition key, then scanned backwards for the same answer. SAN and UCI strings were
    // never read here.
    let mut found = books
        .get(repetition_key(&rules::make_fen(&pos)))
        .map(|&index| &named[index]);
    for text in &moves[..end] {
        let Some(m) = rules::parse_uci(text.as_ref()).and_then(|u| rules::legal_move(&pos, u))
        else {
            break;
        };
        pos.play_unchecked(m);
        if let Some(&index) = books.get(repetition_key(&rules::make_fen(&pos))) {
            found = Some(&named[index]);
        }
    }
    found.map(|line| OpeningName {
        eco: line.eco.clone(),
        name: line.name.clone(),
    })
}

/// `OpeningNode`: one entry of the openings tree. A family (`King's Gambit`) holds the families
/// named after it (`King's Gambit Accepted`, labelled `Accepted`) and its variations, which hold
/// their subvariations. An entry no line is named for exactly has no `eco` or `moves`.
#[derive(Debug, Serialize)]
pub struct OpeningNode {
    /// The full name, as `openingAt` gives it.
    pub name: String,
    /// The part of the name below the parent entry.
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eco: Option<String>,
    /// The shortest line with this name, as UCI moves from the starting position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
    pub children: Vec<OpeningNode>,
}

impl OpeningNode {
    fn new(name: String, label: String) -> OpeningNode {
        OpeningNode {
            name,
            label,
            eco: None,
            moves: None,
            children: Vec::new(),
        }
    }
    fn child(&mut self, name: String, label: &str) -> &mut OpeningNode {
        let at = match self.children.iter().position(|child| child.name == name) {
            Some(at) => at,
            None => {
                self.children
                    .push(OpeningNode::new(name, label.to_string()));
                self.children.len() - 1
            }
        };
        &mut self.children[at]
    }
    fn sort(&mut self) {
        self.children
            .sort_by_cached_key(|child| child.label.to_lowercase());
        self.children.iter_mut().for_each(OpeningNode::sort);
    }
}

/// `openingTree`: every named opening by family, alphabetically at each level.
pub fn opening_tree() -> &'static Vec<OpeningNode> {
    static TREE: OnceLock<Vec<OpeningNode>> = OnceLock::new();
    TREE.get_or_init(build_opening_tree)
}

fn build_opening_tree() -> Vec<OpeningNode> {
    let mut families: Vec<OpeningNode> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for line in lines() {
        let (family, variations) = match line.name.split_once(": ") {
            Some((family, rest)) => (family, rest.split(", ").collect::<Vec<_>>()),
            None => (line.name.as_str(), Vec::new()),
        };
        let at = *index.entry(family.to_string()).or_insert_with(|| {
            families.push(OpeningNode::new(family.to_string(), family.to_string()));
            families.len() - 1
        });
        let mut node = &mut families[at];
        for (depth, label) in variations.iter().enumerate() {
            let name = format!("{family}: {}", variations[..=depth].join(", "));
            node = node.child(name, label);
        }
        // Several lines may share a name; the shortest is the one the entry opens.
        if node
            .moves
            .as_ref()
            .is_none_or(|known| line.moves.len() < known.len())
        {
            node.eco = Some(line.eco.clone());
            node.moves = Some(line.moves.clone());
        }
    }
    // A family named after another one (`King's Gambit Accepted`) goes under the longest such.
    let names: Vec<String> = families.iter().map(|family| family.name.clone()).collect();
    let parent_of = |name: &str| {
        names
            .iter()
            .filter(|other| {
                name.len() > other.len()
                    && name.starts_with(other.as_str())
                    && name[other.len()..].starts_with(' ')
            })
            .max_by_key(|other| other.len())
            .cloned()
    };
    let parents: Vec<Option<String>> = names.iter().map(|name| parent_of(name)).collect();
    // Longest names first, so a family has its own sub-families before it moves under its parent.
    let mut order: Vec<usize> = (0..families.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(names[i].len()));
    let mut slots: Vec<Option<OpeningNode>> = families.into_iter().map(Some).collect();
    for i in order {
        let Some(parent) = &parents[i] else { continue };
        let Some(mut family) = slots[i].take() else {
            continue;
        };
        family.label = family.name[parent.len() + 1..].to_string();
        if let Some(target) = slots[index[parent]].as_mut() {
            target.children.push(family);
        }
    }
    let mut roots: Vec<OpeningNode> = slots.into_iter().flatten().collect();
    roots.sort_by_cached_key(|root| root.name.to_lowercase());
    roots.iter_mut().for_each(OpeningNode::sort);
    roots
}

/// The longest line with a Wikibooks page title (deeper pages are rare).
const WIKIBOOKS_PLIES: usize = 40;

/// The Wikibooks “Chess Opening Theory” page of a line from the starting position, as Lichess
/// names it: `Chess_Opening_Theory/1._e4/1...e5/2._f4`. None for an empty, illegal or very long line.
pub fn wikibooks_title<S: AsRef<str>>(moves: &[S]) -> Option<String> {
    if moves.is_empty() || moves.len() > WIKIBOOKS_PLIES {
        return None;
    }
    let start = rules::setup_start("standard", START)?;
    let (_, line) = position::line(start, moves, false, false);
    if line.len() != moves.len() {
        return None;
    }
    let mut title = String::from("Chess_Opening_Theory");
    for (ply, step) in line.iter().enumerate() {
        let number = ply / 2 + 1;
        let dots = if ply % 2 == 0 { "._" } else { "..." };
        title.push_str(&format!("/{number}{dots}{}", step.san));
    }
    Some(title)
}

/// The positions (FENs) a line from the starting position reaches, the start first. None when a
/// move is illegal.
pub fn line_fens<S: AsRef<str>>(moves: &[S]) -> Option<Vec<String>> {
    let start = rules::setup_start("standard", START)?;
    let first = rules::make_fen(&start);
    let (_, line) = position::line(start, moves, false, false);
    if line.len() != moves.len() {
        return None;
    }
    Some(
        std::iter::once(first)
            .chain(line.into_iter().map(|step| step.fen))
            .collect(),
    )
}

/// `openingKey`: the key Lichess names an opening's puzzles by: its family and first variation
/// (the depth of puzzles' `OpeningTags`), without accents or punctuation, spaces as underscores.
/// `King's Gambit Accepted: Kieseritzky Gambit, Long Whip` → `Kings_Gambit_Accepted_Kieseritzky_Gambit`.
pub fn opening_key(name: &str) -> String {
    let (family, rest) = name.split_once(": ").unwrap_or((name, ""));
    let named = match rest
        .split(", ")
        .next()
        .filter(|variation| !variation.is_empty())
    {
        Some(variation) => format!("{family} {variation}"),
        None => family.to_string(),
    };
    let plain: String = named
        .chars()
        .filter_map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => Some('a'),
            'é' | 'è' | 'ê' | 'ë' => Some('e'),
            'í' | 'ì' | 'î' | 'ï' => Some('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => Some('o'),
            'ú' | 'ù' | 'û' | 'ü' => Some('u'),
            'ć' | 'ç' | 'č' => Some('c'),
            'ñ' | 'ń' => Some('n'),
            c if c.is_ascii_alphanumeric() || c == '-' || c == ' ' => Some(c),
            _ => None,
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join("_")
}

/// A move from an opening explorer, as `LookupMove` gives it.
#[derive(Debug, Deserialize)]
pub struct ExplorerMove {
    pub uci: String,
    #[serde(default)]
    pub white: f64,
    #[serde(default)]
    pub draws: f64,
    #[serde(default)]
    pub black: f64,
}

/// The replies a drill of an opening asks for or plays beyond its named line.
const REPLIES_MAX: usize = 3;
/// The opponent's replies cover this share of the games…
const REPLIES_COVER: f64 = 0.8;
/// …each played in at least this share of them.
const REPLY_SHARE_MIN: f64 = 0.05;

/// `openingReplies`: the moves an opening drill continues with from a position the explorer knows.
/// The player is expected to play the most played move; the opponent answers with the popular
/// replies (up to three, until they cover most games, none rarer than one game in twenty).
pub fn opening_replies(moves: &[ExplorerMove], yours: bool) -> Vec<String> {
    let games = |entry: &ExplorerMove| entry.white + entry.draws + entry.black;
    let mut played: Vec<&ExplorerMove> = moves.iter().filter(|entry| games(entry) > 0.0).collect();
    // Stable: equally played moves keep the explorer's order.
    played.sort_by(|a, b| games(b).total_cmp(&games(a)));
    let total: f64 = played.iter().map(|entry| games(entry)).sum();
    if yours {
        return played
            .first()
            .map(|entry| entry.uci.clone())
            .into_iter()
            .collect();
    }
    let mut picked = Vec::new();
    let mut covered = 0.0;
    for entry in played {
        let share = games(entry) / total;
        if picked.len() >= REPLIES_MAX
            || (!picked.is_empty() && (covered >= REPLIES_COVER || share < REPLY_SHARE_MIN))
        {
            break;
        }
        covered += share;
        picked.push(entry.uci.clone());
    }
    picked
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "openingAt" => opening_at_call(args),
        "openingTree" => to_value(opening_tree()),
        "openingReplies" => opening_replies_call(args),
        "openingKey" => {
            super::text_arg(args, 0, "name").and_then(|name| to_value(opening_key(name)))
        }
        _ => return None,
    };
    Some(result)
}

fn opening_at_call(args: &[Value]) -> Out {
    let setup: Value = value_arg(args, 0, "setup")?;
    let variant = setup.get("variant").and_then(Value::as_str).unwrap_or("");
    let fen = setup.get("fen").and_then(Value::as_str).unwrap_or("");
    let moves: Vec<String> = value_arg(args, 1, "moves")?;
    // `ply` defaults to the number of moves; a non-finite one arrives as its text.
    let ply = match args.get(2).filter(|v| !v.is_null()) {
        Some(_) => js::to_number(args.get(2)),
        None => moves.len() as f64,
    };
    to_value(opening_at(variant, fen, &moves, ply))
}

fn opening_replies_call(args: &[Value]) -> Out {
    let moves: Vec<ExplorerMove> = value_arg(args, 0, "moves")?;
    let yours = super::arg(args, 1)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    to_value(opening_replies(&moves, yours))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn names_the_last_named_position() {
        let moves: Vec<String> = ["e2e4", "a7a6", "h2h3"]
            .iter()
            .map(|m| (*m).to_string())
            .collect();
        // After 1.e4 a6 2.h3 the nearest named position is 1.e4 a6 (St. George Defense).
        let name = opening_at("standard", START, &moves, 3.0).expect("a named opening");
        assert_eq!(
            (name.eco.as_str(), name.name.as_str()),
            ("B00", "St. George Defense")
        );
        let first = opening_at("standard", START, &moves, 1.0).expect("the first move is named");
        assert_eq!(first.name, "King's Pawn Game");
        assert!(opening_at("chess960", START, &moves, 1.0).is_none());
        assert!(opening_at("standard", START, &moves, 0.0).is_none());
    }

    fn find<'a>(nodes: &'a [OpeningNode], label: &str) -> &'a OpeningNode {
        nodes
            .iter()
            .find(|node| node.label == label)
            .unwrap_or_else(|| panic!("{label} is in the tree"))
    }

    #[test]
    fn groups_openings_by_family() {
        let tree = opening_tree();
        let gambit = find(tree, "King's Gambit");
        assert_eq!(gambit.eco.as_deref(), Some("C30"));
        assert_eq!(
            gambit.moves.as_deref(),
            Some(&["e2e4", "e7e5", "f2f4"].map(String::from)[..])
        );
        // Families named after it sit below it, labelled by the rest of their name.
        let accepted = find(&gambit.children, "Accepted");
        assert_eq!(accepted.name, "King's Gambit Accepted");
        let kieseritzky = find(&accepted.children, "Kieseritzky Gambit");
        assert_eq!(
            kieseritzky.name,
            "King's Gambit Accepted: Kieseritzky Gambit"
        );
        // No line is named just “King's Gambit Declined”: the entry only groups its variations.
        let declined = find(&gambit.children, "Declined");
        assert!(declined.moves.is_none() && !declined.children.is_empty());
        assert!(
            !tree
                .iter()
                .any(|root| root.name == "King's Gambit Accepted")
        );
        // Every line's name is reachable, and every entry with moves is named after them.
        fn count(nodes: &[OpeningNode]) -> usize {
            nodes
                .iter()
                .map(|node| usize::from(node.moves.is_some()) + count(&node.children))
                .sum()
        }
        let names: std::collections::HashSet<&str> =
            lines().iter().map(|line| line.name.as_str()).collect();
        assert_eq!(count(tree), names.len());
        let labels: Vec<&str> = tree.iter().map(|root| root.label.as_str()).collect();
        let mut sorted = labels.clone();
        sorted.sort_by_key(|label| label.to_lowercase());
        assert_eq!(labels, sorted);
    }

    #[test]
    fn names_wikibooks_pages_like_lichess() {
        let moves = ["e2e4", "e7e5", "f2f4", "e5f4", "g1f3"];
        assert_eq!(
            wikibooks_title(&moves).as_deref(),
            Some("Chess_Opening_Theory/1._e4/1...e5/2._f4/2...exf4/3._Nf3")
        );
        assert_eq!(
            wikibooks_title(&["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1g1"]).as_deref(),
            Some("Chess_Opening_Theory/1._e4/1...e5/2._Nf3/2...Nc6/3._Bc4/3...Nf6/4._O-O")
        );
        assert!(wikibooks_title::<&str>(&[]).is_none());
        assert!(wikibooks_title(&["e2e5"]).is_none());
        let fens = line_fens(&["e2e4"]).expect("legal");
        assert_eq!(fens.len(), 2);
        assert_eq!(fens[0], START);
        assert!(line_fens(&["e2e4", "e2e4"]).is_none());
    }

    #[test]
    fn keys_openings_as_lichess_puzzles_do() {
        assert_eq!(
            opening_key("King's Gambit Accepted: Kieseritzky Gambit, Long Whip"),
            "Kings_Gambit_Accepted_Kieseritzky_Gambit"
        );
        assert_eq!(opening_key("King's Gambit"), "Kings_Gambit");
        assert_eq!(
            opening_key("Caro-Kann Defense: Advance Variation"),
            "Caro-Kann_Defense_Advance_Variation"
        );
        assert_eq!(opening_key("Réti Opening"), "Reti_Opening");
    }

    #[test]
    fn picks_popular_replies() {
        let moves = |games: &[(&str, f64)]| -> Vec<ExplorerMove> {
            games
                .iter()
                .map(|(uci, white)| ExplorerMove {
                    uci: (*uci).to_string(),
                    white: *white,
                    draws: 0.0,
                    black: 0.0,
                })
                .collect()
        };
        let stats = moves(&[
            ("a", 10.0),
            ("b", 60.0),
            ("c", 25.0),
            ("d", 4.0),
            ("e", 1.0),
        ]);
        assert_eq!(opening_replies(&stats, true), ["b"]);
        // b and c cover 85%: enough.
        assert_eq!(opening_replies(&stats, false), ["b", "c"]);
        let even = moves(&[("a", 30.0), ("b", 30.0), ("c", 30.0), ("d", 10.0)]);
        assert_eq!(opening_replies(&even, false), ["a", "b", "c"]);
        let rare = moves(&[("a", 70.0), ("b", 3.0), ("c", 0.0)]);
        assert_eq!(opening_replies(&rare, false), ["a"]);
        assert!(opening_replies(&moves(&[("a", 0.0)]), false).is_empty());
        assert!(opening_replies(&[], true).is_empty());
    }
}
