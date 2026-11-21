//! The openings the page lists: each one's name, code, moves and position, their groups, and
//! a search through them.

use super::*;

/// An opening as a card shows it: its name and code, its position and its moves.
#[derive(Clone)]
pub(super) struct Entry {
    pub(super) name: String,
    pub(super) eco: String,
    pub(super) moves: Vec<String>,
    pub(super) fen: String,
    pub(super) san: String,
}

/// `percent`: a share of games, to one decimal under 10%.
pub(super) fn percent(value: f64) -> String {
    if value < 10. {
        format!("{value:.1}%")
    } else {
        format!("{}%", value.round())
    }
}

/// The moves as far as they are legal, each with its SAN and the position after it.
pub(super) fn line_of(moves: &[String]) -> Vec<(String, String, String)> {
    let Some(replay) = kchess_domain::replay::replay_setup(
        kchess_domain::variant::Variant::Standard,
        STANDARD_FEN,
        moves,
    ) else {
        return Vec::new();
    };
    let positions = kchess_domain::replay::replay_positions(STANDARD_FEN, moves);
    replay
        .played
        .into_iter()
        .zip(positions.into_iter().skip(1))
        .map(|(played, position)| (played.uci, played.san, position.fen))
        .collect()
}

/// The name and code of the line.
pub(super) fn opening_at(moves: &[String]) -> Option<(String, String)> {
    let named = kchess_domain::training::openings::opening_at(
        "standard",
        STANDARD_FEN,
        moves,
        moves.len() as f64,
    )?;
    Some((named.name, named.eco))
}

pub(super) fn entry(moves: &[String], name: Option<&str>) -> Option<Entry> {
    let steps = line_of(moves);
    let (_, _, fen) = steps.last()?.clone();
    let line: Vec<String> = steps.iter().map(|(uci, _, _)| uci.clone()).collect();
    let (opening, eco) = opening_at(&line)?;
    let san = steps
        .iter()
        .enumerate()
        .map(|(index, (_, san, _))| {
            if index % 2 == 0 {
                format!("{}.{san}", index / 2 + 1)
            } else {
                san.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(Entry {
        name: name.map_or(opening, str::to_string),
        eco,
        moves: line,
        fen,
        san,
    })
}

/// Search ignores case, accents and the kind of apostrophe.
pub(super) fn fold(text: &str) -> String {
    // Lower case first, so a capital with an accent (É) loses its accent too.
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|letter| match letter {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            '’' | '`' => '\'',
            letter => letter,
        })
        .collect()
}

/// One character as `fold` reads it, for lighting what the search found: one that folds to
/// several characters stays as it is.
pub(super) fn fold_char(letter: char) -> char {
    let folded = fold(letter.encode_utf8(&mut [0; 4]));
    let mut chars = folded.chars();
    match (chars.next(), chars.next()) {
        (Some(one), None) => one,
        _ => letter,
    }
}

pub(super) fn first_move(node: &OpeningNode) -> Option<&str> {
    node.moves
        .as_ref()
        .and_then(|moves| moves.first().map(String::as_str))
        .or_else(|| node.children.iter().find_map(first_move))
}

pub(super) fn group_of(node: &OpeningNode) -> &'static str {
    let first = first_move(node);
    GROUPS
        .iter()
        .find(|(_, _, moves)| first.is_some_and(|first| moves.contains(&first)))
        .map_or("other", |(key, _, _)| key)
}

/// The named openings, shortest line first, with their folded search text.
pub(super) fn named(tree: &[OpeningNode]) -> Vec<(String, String, Vec<String>, String)> {
    fn walk(nodes: &[OpeningNode], out: &mut Vec<(String, String, Vec<String>, String)>) {
        for node in nodes {
            if let Some(moves) = &node.moves {
                let eco = node.eco.clone().unwrap_or_default();
                out.push((
                    node.name.clone(),
                    eco.clone(),
                    moves.clone(),
                    fold(&format!("{} {eco}", node.name)),
                ));
            }
            walk(&node.children, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out.sort_by(|a, b| a.2.len().cmp(&b.2.len()).then_with(|| a.3.cmp(&b.3)));
    out
}

/// The openings the browser lists, the same for every page: the rules' tree, its named lines
/// and the popular ones (`catalog`).
pub(super) struct Catalog {
    pub(super) tree: &'static [OpeningNode],
    pub(super) named: Vec<(String, String, Vec<String>, String)>,
    pub(super) popular: Vec<Entry>,
}

/// The catalog, made the first time a page asks for it and shared by every page after.
pub(super) fn catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let tree = opening_tree();
            let named = named(tree);
            let popular = POPULAR
                .iter()
                .filter_map(|line| {
                    let moves: Vec<String> = line.split(' ').map(str::to_string).collect();
                    entry(&moves, None)
                })
                .collect();
            Arc::new(Catalog {
                tree,
                named,
                popular,
            })
        })
        .clone()
}
