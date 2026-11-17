//! Quick-pick search: prefix modes and a VS Code-style fuzzy matcher with
//! the ranges to highlight.

/// What the palette searches, from the typed text's first character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    All,
    Commands,
    Players,
    Settings,
    Help,
}

/// The mode a leading prefix picks, and the text to match.
pub fn parse(raw: &str) -> (Mode, &str) {
    let mode = match raw.chars().next() {
        Some('>') => Mode::Commands,
        Some('@') => Mode::Players,
        Some('#') => Mode::Settings,
        Some('?') => Mode::Help,
        _ => return (Mode::All, raw.trim()),
    };
    (mode, raw[1..].trim())
}

/// A match: its score and the inclusive character ranges of the label to highlight.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub score: f64,
    pub ranges: Vec<(usize, usize)>,
}

fn is_break(c: char) -> bool {
    c.is_whitespace() || "·›/@:,()+-".contains(c)
}

fn is_word_start(text: &[char], index: usize) -> bool {
    index == 0 || is_break(text[index - 1])
}

fn find(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&at| haystack[at..at + needle.len()] == *needle)
}

/// One token in `text`: whole substrings beat acronym-style subsequences, word starts beat the
/// middle.
fn match_token(token: &[char], text: &str) -> Option<Match> {
    let original: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    if lower.len() != original.len() {
        return None;
    }
    let mut fallback = None;
    let mut at = find(&lower, token, 0);
    while let Some(index) = at {
        if is_word_start(&original, index) {
            return Some(Match {
                score: 100. + token.len() as f64 * 4. + if index == 0 { 40. } else { 0. },
                ranges: vec![(index, index + token.len() - 1)],
            });
        }
        fallback.get_or_insert(index);
        at = find(&lower, token, index + 1);
    }
    if let Some(index) = fallback
        && token.len() > 1
    {
        return Some(Match {
            score: 50. + token.len() as f64 * 2.,
            ranges: vec![(index, index + token.len() - 1)],
        });
    }
    // A subsequence starting at a word: "otb" → Over The Board.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut score = 20.;
    let mut from = 0;
    for &c in token {
        let word = (from..lower.len()).find(|&i| lower[i] == c && is_word_start(&original, i));
        let next = (from..lower.len()).find(|&i| lower[i] == c);
        let contiguous = !ranges.is_empty() && next == Some(from);
        let at = if contiguous || word.is_none() {
            next
        } else {
            word
        }?;
        if ranges.is_empty() && !is_word_start(&original, at) {
            return None;
        }
        match ranges.last_mut() {
            Some(last) if last.1 + 1 == at => {
                last.1 = at;
                score += 6.;
            }
            _ => {
                ranges.push((at, at));
                if is_word_start(&original, at) {
                    score += 8.;
                }
            }
        }
        from = at + 1;
    }
    Some(Match { score, ranges })
}

fn merge(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_by_key(|range| range.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.0 <= last.1 + 1 => last.1 = last.1.max(range.1),
            _ => merged.push(range),
        }
    }
    merged
}

/// What a query naming the whole label adds, above any longer label it also starts.
const EXACT_LABEL: f64 = 100.;

/// Every token must match the label or the extra text (keywords, details); only label matches are
/// highlighted, and they score higher.
pub fn match_query(query: &str, label: &str, extra: &str) -> Option<Match> {
    let tokens: Vec<Vec<char>> = query
        .to_lowercase()
        .split_whitespace()
        .map(|token| token.chars().collect())
        .collect();
    if tokens.is_empty() {
        return Some(Match {
            score: 0.,
            ranges: Vec::new(),
        });
    }
    let mut score = 0.;
    let mut ranges = Vec::new();
    for token in &tokens {
        if let Some(found) = match_token(token, label) {
            score += found.score;
            ranges.extend(found.ranges);
            continue;
        }
        let found = match_token(token, extra)?;
        score += found.score / 2.;
    }
    // Typing an entry's whole name finds it first: "Appearance" before "Appearance: Dark".
    if query.trim().to_lowercase() == label.to_lowercase() {
        score += EXACT_LABEL;
    }
    Some(Match {
        score,
        ranges: merge(ranges),
    })
}

/// The matched characters' byte ranges in `text`, for a `StyledText` to highlight.
pub fn byte_ranges(text: &str, ranges: &[(usize, usize)]) -> Vec<std::ops::Range<usize>> {
    let starts: Vec<usize> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain([text.len()])
        .collect();
    ranges
        .iter()
        .filter(|(start, end)| start <= end && end + 1 < starts.len())
        .map(|&(start, end)| starts[start]..starts[end + 1])
        .collect()
}

/// Where a list's search finds each of `words` in `text`, as the inclusive character ranges
/// `Match` has: every place, compared character by character as `fold` reads them (`words` come
/// folded the same way). A list that finds its rows by plain substrings shows why each row is
/// there with these.
pub fn found(text: &str, words: &[&str], fold: impl Fn(char) -> char) -> Vec<(usize, usize)> {
    let folded: Vec<char> = text.chars().map(&fold).collect();
    let mut ranges = Vec::new();
    for word in words {
        let word: Vec<char> = word.chars().collect();
        let mut from = 0;
        while let Some(at) = find(&folded, &word, from) {
            ranges.push((at, at + word.len() - 1));
            from = at + word.len();
        }
    }
    merge(ranges)
}

/// A character in lower case, as the lists' searches compare them: one that lowers to several
/// characters (İ) stays as it is, so the ranges keep to the text's own characters.
pub fn lower(c: char) -> char {
    let mut lowered = c.to_lowercase();
    match (lowered.next(), lowered.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// Most recent first, without repeats, capped.
pub fn push_recent(recent: &[String], id: &str, limit: usize) -> Vec<String> {
    std::iter::once(id.to_string())
        .chain(recent.iter().filter(|entry| *entry != id).cloned())
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Mode, byte_ranges, found, lower, match_query, parse, push_recent};

    #[test]
    fn prefixes_pick_the_mode() {
        assert_eq!(parse(">new"), (Mode::Commands, "new"));
        assert_eq!(parse("@ bob "), (Mode::Players, "bob"));
        assert_eq!(parse("#zen"), (Mode::Settings, "zen"));
        assert_eq!(parse("?"), (Mode::Help, ""));
        assert_eq!(parse(" puzzles "), (Mode::All, "puzzles"));
    }

    #[test]
    fn word_starts_beat_the_middle_and_acronyms_work() {
        let start = match_query("puz", "Puzzles", "").unwrap();
        let middle = match_query("zle", "Puzzles", "").unwrap();
        assert!(start.score > middle.score);
        assert_eq!(start.ranges, vec![(0, 2)]);
        let acronym = match_query("otb", "Over the board", "").unwrap();
        assert_eq!(acronym.ranges, vec![(0, 0), (5, 5), (9, 9)]);
        assert!(match_query("xyz", "Puzzles", "").is_none());
    }

    #[test]
    fn a_whole_name_beats_longer_labels_it_starts() {
        let exact = match_query("appearance", "Appearance", "Settings").unwrap();
        let longer = match_query("appearance", "Appearance: System", "").unwrap();
        assert!(exact.score > longer.score);
    }

    #[test]
    fn keywords_find_but_are_not_highlighted() {
        let found = match_query("stockfish", "Play with computer", "stockfish bot").unwrap();
        assert!(found.ranges.is_empty());
        assert!(found.score > 0.);
        let both = match_query("play bot", "Play with computer", "stockfish bot").unwrap();
        assert_eq!(both.ranges, vec![(0, 3)]);
    }

    #[test]
    fn matched_characters_become_byte_ranges() {
        assert_eq!(byte_ranges("Puzzles", &[(0, 2)]), vec![0..3]);
        // "é" takes two bytes: the ranges count characters, the result bytes.
        assert_eq!(byte_ranges("Réti", &[(1, 2)]), vec![1..4]);
        assert_eq!(byte_ranges("abc", &[(0, 0), (2, 2)]), vec![0..1, 2..3]);
        // A range past the text is left out rather than cutting a character.
        assert!(byte_ranges("ab", &[(1, 2)]).is_empty());
    }

    #[test]
    fn a_list_search_finds_every_place_of_each_word() {
        assert_eq!(found("Bob vs bobby", &["bob"], lower), vec![(0, 2), (7, 9)]);
        assert_eq!(
            found("Sicilian Defense", &["def", "sic"], lower),
            vec![(0, 2), (9, 11)]
        );
        // Overlapping and touching places join into one range.
        assert_eq!(found("aaaa", &["aa", "aaa"], lower), vec![(0, 3)]);
        assert!(found("Sicilian", &["xyz"], lower).is_empty());
        assert!(found("Sicilian", &[], lower).is_empty());
        assert!(found("Sicilian", &[""], lower).is_empty());
    }

    #[test]
    fn lowering_keeps_one_character_for_each() {
        assert_eq!(lower('A'), 'a');
        assert_eq!(lower('é'), 'é');
        assert_eq!(lower('\u{130}'), '\u{130}');
    }

    #[test]
    fn recent_entries_come_first_once() {
        let recent = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(push_recent(&recent, "b", 2), vec!["b", "a"]);
    }
}
