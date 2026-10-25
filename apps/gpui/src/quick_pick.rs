//! Quick-pick search (`utils/quickPick.ts`): prefix modes and a VS Code-style fuzzy matcher with
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

/// The mode a leading prefix picks, and the text to match (`parseQuickPick`).
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
/// middle (`matchToken`).
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

/// Every token must match the label or the extra text (keywords, details); only label matches are
/// highlighted, and they score higher (`matchQuickPick`).
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
    Some(Match {
        score,
        ranges: merge(ranges),
    })
}

/// The label split at the matched ranges: `(text, matched)` runs.
pub fn segments(label: &str, ranges: &[(usize, usize)]) -> Vec<(String, bool)> {
    let chars: Vec<char> = label.chars().collect();
    let mut out = Vec::new();
    let mut push = |from: usize, to: usize, mark: bool| {
        if from < to {
            out.push((chars[from..to].iter().collect(), mark));
        }
    };
    let mut from = 0;
    for &(start, end) in ranges {
        push(from, start, false);
        push(start, end + 1, true);
        from = end + 1;
    }
    push(from, chars.len(), false);
    out
}

/// Most recent first, without repeats, capped (`pushRecent`).
pub fn push_recent(recent: &[String], id: &str, limit: usize) -> Vec<String> {
    std::iter::once(id.to_string())
        .chain(recent.iter().filter(|entry| *entry != id).cloned())
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Mode, match_query, parse, push_recent, segments};

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
    fn keywords_find_but_are_not_highlighted() {
        let found = match_query("stockfish", "Play with Computer", "stockfish bot").unwrap();
        assert!(found.ranges.is_empty());
        assert!(found.score > 0.);
        let both = match_query("play bot", "Play with Computer", "stockfish bot").unwrap();
        assert_eq!(both.ranges, vec![(0, 3)]);
    }

    #[test]
    fn labels_split_at_their_matches() {
        assert_eq!(
            segments("Puzzles", &[(0, 2)]),
            vec![("Puz".to_string(), true), ("zles".to_string(), false)]
        );
    }

    #[test]
    fn recent_entries_come_first_once() {
        let recent = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(push_recent(&recent, "b", 2), vec!["b", "a"]);
    }
}
