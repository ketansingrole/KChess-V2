//! Spaced repetition for repertoire lines: an SM-2 schedule per line of a repertoire.
//!
//! A repertoire (a study or an opening, and the side played: the trainer's miss key) has lines,
//! each the moves from a tree's root to one of its leaves. Each line drilled gets a card: its
//! interval in days, ease factor and due time. A line finished without a mistake is answered
//! right, anything else wrong. The schedule is a versioned library document,
//! `{ "version": 1, "lines": { key: { line: card } } }`; anything else reads as no schedule, so
//! every line of an older profile is new.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::js;

pub const DAY_MS: f64 = 86_400_000.0;
/// A new line's ease, and the most it recovers to.
pub const START_EASE: f64 = 2.5;
pub const MIN_EASE: f64 = 1.3;
pub const MAX_INTERVAL_DAYS: f64 = 365.0;
pub const SCHEDULE_VERSION: u64 = 1;
/// Repertoires (keys) kept, lines per repertoire, and a key's and a line's longest text.
pub const MAX_KEYS: usize = 200;
pub const MAX_LINES: usize = 2000;
pub const MAX_KEY_CHARS: usize = 200;
pub const MAX_LINE_CHARS: usize = 1300;

/// One line's place in the schedule.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// Days until the next review after a right answer; 0 after a lapse.
    pub interval: f64,
    pub ease: f64,
    /// When the line is next due, in milliseconds since the epoch.
    pub due: f64,
    /// Right answers in a row.
    pub reps: u32,
    /// Times the line was answered wrong after being learned or while new.
    pub lapses: u32,
}

impl Card {
    pub fn is_due(&self, now: f64) -> bool {
        self.due <= now
    }

    /// The card after an answer. A wrong answer lapses the line: it is due again at once and its
    /// ease falls. A right answer on a due (or new) line moves it out 1 day, then 6, then by its
    /// ease; a right answer before the line is due leaves the schedule as it was, so drilling
    /// at will does not push lines out.
    pub fn review(previous: Option<&Card>, correct: bool, now: f64) -> Card {
        let card = previous.copied().unwrap_or(Card {
            interval: 0.0,
            ease: START_EASE,
            due: now,
            reps: 0,
            lapses: 0,
        });
        if !correct {
            return Card {
                interval: 0.0,
                ease: (card.ease - 0.2).max(MIN_EASE),
                due: now,
                reps: 0,
                lapses: card.lapses.saturating_add(1),
            };
        }
        if previous.is_some() && !card.is_due(now) {
            return card;
        }
        let reps = card.reps.saturating_add(1);
        let interval = match reps {
            1 => 1.0,
            2 => 6.0,
            _ => (card.interval.max(1.0) * card.ease).round(),
        }
        .min(MAX_INTERVAL_DAYS);
        Card {
            interval,
            ease: (card.ease + 0.1).min(START_EASE),
            due: now + interval * DAY_MS,
            reps,
            lapses: card.lapses,
        }
    }

    fn decode(raw: &Value) -> Option<Card> {
        let field = |name: &str| {
            raw.get(name)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
        };
        let whole = |name: &str| {
            field(name)
                .filter(|n| *n >= 0.0 && n.fract() == 0.0)
                .map(|n| n.min(f64::from(u32::MAX)) as u32)
        };
        let card = Card {
            interval: field("interval").filter(|n| (0.0..=MAX_INTERVAL_DAYS).contains(n))?,
            ease: field("ease").filter(|n| (MIN_EASE..=START_EASE).contains(n))?,
            due: field("due").filter(|n| *n >= 0.0)?,
            reps: whole("reps")?,
            lapses: whole("lapses")?,
        };
        Some(card)
    }

    fn to_value(self) -> Value {
        json!({
            "interval": self.interval,
            "ease": self.ease,
            "due": self.due,
            "reps": self.reps,
            "lapses": self.lapses,
        })
    }
}

/// A line's name: the UCI moves from its root, space-separated, after `<8 hex digits>:` (the
/// root position's [`root_tag`]) when the tree does not start from the standard position.
pub fn valid_line(line: &str) -> bool {
    let moves = match line.split_once(':') {
        Some((tag, moves)) => {
            if tag.len() != 8 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
                return false;
            }
            moves
        }
        None => line,
    };
    let uci = |m: &str| {
        let b = m.as_bytes();
        (b.len() == 4 || (b.len() == 5 && b"qrbnk".contains(&b[4])))
            && (b'a'..=b'h').contains(&b[0])
            && (b'1'..=b'8').contains(&b[1])
            && (b'a'..=b'h').contains(&b[2])
            && (b'1'..=b'8').contains(&b[3])
    };
    !moves.is_empty() && line.len() <= MAX_LINE_CHARS && moves.split(' ').all(uci)
}

/// The tag a line from a non-standard start carries: FNV-1a of the root FEN, as 8 hex digits.
pub fn root_tag(fen: &str) -> String {
    let hash = fen.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    format!("{hash:08x}")
}

/// A line's name from its root position and its moves' path.
pub fn line_id(root_fen: &str, standard: bool, path: &str) -> String {
    if standard {
        path.to_string()
    } else {
        format!("{}:{path}", root_tag(root_fen))
    }
}

/// The stored schedule document's lines, `key` → line → card; anything unreadable is dropped.
pub fn decode_schedule(raw: &Value) -> Map<String, Value> {
    let mut result = Map::new();
    if raw.get("version").and_then(Value::as_u64) != Some(SCHEDULE_VERSION) {
        return result;
    }
    let Some(keys) = raw.get("lines").and_then(Value::as_object) else {
        return result;
    };
    for (key, table) in keys.iter().take(MAX_KEYS) {
        let Some(table) = table.as_object() else {
            continue;
        };
        if key.is_empty() || js::utf16_len(key) > MAX_KEY_CHARS {
            continue;
        }
        let cards: Map<String, Value> = table
            .iter()
            .take(MAX_LINES)
            .filter(|(line, _)| valid_line(line))
            .filter_map(|(line, card)| Some((line.clone(), Card::decode(card)?.to_value())))
            .collect();
        result.insert(key.clone(), Value::Object(cards));
    }
    result
}

/// The schedule's lines as the document to store.
pub fn encode_schedule(lines: Map<String, Value>) -> Value {
    json!({ "version": SCHEDULE_VERSION, "lines": lines })
}

/// The schedule after answering one line. A repertoire at [`MAX_LINES`] makes room by dropping
/// the line due last; a new repertoire past [`MAX_KEYS`] is refused.
pub fn record(
    schedule: &mut Map<String, Value>,
    key: &str,
    line: &str,
    correct: bool,
    now: f64,
) -> Result<Card, String> {
    if key.is_empty() || js::utf16_len(key) > MAX_KEY_CHARS {
        return Err("A repertoire's name must be 1 to 200 characters.".into());
    }
    if !valid_line(line) {
        return Err("A repertoire line must be its UCI moves.".into());
    }
    if !schedule.contains_key(key) && schedule.len() >= MAX_KEYS {
        return Err("Too many repertoires are scheduled.".into());
    }
    let table = schedule.entry(key.to_string()).or_insert_with(|| json!({}));
    if !table.is_object() {
        *table = json!({});
    }
    let Some(table) = table.as_object_mut() else {
        return Err("A repertoire's schedule must be an object.".into());
    };
    let previous = table.get(line).and_then(Card::decode);
    if previous.is_none() && table.len() >= MAX_LINES {
        let last = table
            .iter()
            .max_by(|a, b| due_of(a.1).total_cmp(&due_of(b.1)))
            .map(|(line, _)| line.clone());
        if let Some(last) = last {
            table.shift_remove(&last);
        }
    }
    let card = Card::review(previous.as_ref(), correct, now);
    table.insert(line.to_string(), card.to_value());
    Ok(card)
}

fn due_of(card: &Value) -> f64 {
    card.get("due").and_then(Value::as_f64).unwrap_or(f64::MAX)
}

/// How a repertoire's lines stand: how many are due, and when the next of the rest is due.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub due: usize,
    /// Lines never answered yet.
    pub new: usize,
    pub next_due: Option<f64>,
}

/// The cards of `lines` in one repertoire's table (`key` → card), as they stand at `now`.
pub fn summary<'a>(lines: impl IntoIterator<Item = &'a str>, table: &Value, now: f64) -> Summary {
    let mut summary = Summary::default();
    for line in lines {
        match table.get(line).and_then(Card::decode) {
            None => summary.new += 1,
            Some(card) if card.is_due(now) => summary.due += 1,
            Some(card) => {
                summary.next_due = Some(summary.next_due.map_or(card.due, |d| d.min(card.due)));
            }
        }
    }
    summary
}

/// The line to review first: the most overdue of `lines`, or None when none is due.
pub fn most_overdue(lines: &[&str], table: &Value, now: f64) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| Some((index, table.get(*line).and_then(Card::decode)?)))
        .filter(|(_, card)| card.is_due(now))
        .min_by(|a, b| a.1.due.total_cmp(&b.1.due))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000_000.0;

    #[test]
    fn right_answers_space_a_line_out_and_a_wrong_one_brings_it_back() {
        let first = Card::review(None, true, NOW);
        assert_eq!(
            (first.reps, first.interval, first.ease),
            (1, 1.0, START_EASE)
        );
        assert_eq!(first.due, NOW + DAY_MS);
        let second = Card::review(Some(&first), true, first.due);
        assert_eq!((second.reps, second.interval), (2, 6.0));
        let third = Card::review(Some(&second), true, second.due);
        assert_eq!((third.reps, third.interval), (3, 15.0));
        assert_eq!(third.due, second.due + 15.0 * DAY_MS);

        let lapse = Card::review(Some(&third), false, third.due + 5.0);
        assert_eq!((lapse.reps, lapse.interval, lapse.lapses), (0, 0.0, 1));
        assert!((lapse.ease - 2.3).abs() < 1e-9);
        // Due again at once, then a day out, with its ease recovering.
        assert!(lapse.is_due(third.due + 5.0));
        let relearned = Card::review(Some(&lapse), true, third.due + 10.0);
        assert_eq!((relearned.reps, relearned.interval), (1, 1.0));
        assert!((relearned.ease - 2.4).abs() < 1e-9);
    }

    #[test]
    fn a_right_answer_before_the_line_is_due_changes_nothing() {
        let card = Card::review(None, true, NOW);
        assert_eq!(Card::review(Some(&card), true, NOW + 60_000.0), card);
        // A wrong one still lapses it.
        assert_eq!(Card::review(Some(&card), false, NOW + 60_000.0).reps, 0);
    }

    #[test]
    fn ease_and_interval_stay_in_bounds() {
        let mut card = Card::review(None, false, NOW);
        for _ in 0..20 {
            card = Card::review(Some(&card), false, NOW);
        }
        assert_eq!(card.ease, MIN_EASE);
        let mut now = NOW;
        for _ in 0..40 {
            card = Card::review(Some(&card), true, now);
            now = card.due;
        }
        assert_eq!(card.interval, MAX_INTERVAL_DAYS);
        assert_eq!(card.ease, START_EASE);
    }

    #[test]
    fn lines_are_their_uci_moves() {
        assert!(valid_line("e2e4 e7e5 g1f3"));
        assert!(valid_line("e7e8q"));
        assert!(valid_line(&format!(
            "{}:e2e4",
            root_tag("8/8/8/8/8/8/8/K6k w - - 0 1")
        )));
        assert!(!valid_line(""));
        assert!(!valid_line("e2e4  e7e5"));
        assert!(!valid_line("e2e9"));
        assert!(!valid_line("xyz:e2e4"));
        assert!(!valid_line(&vec!["e2e4"; 300].join(" ")));
        assert_eq!(root_tag("a"), "e40c292c");
        assert_eq!(line_id("x", true, "e2e4"), "e2e4");
    }

    #[test]
    fn an_old_or_damaged_document_is_no_schedule() {
        assert!(decode_schedule(&Value::Null).is_empty());
        assert!(decode_schedule(&json!({ "white": {} })).is_empty());
        assert!(decode_schedule(&json!({ "version": 2, "lines": {} })).is_empty());
        let card = Card::review(None, true, NOW).to_value();
        let raw = json!({ "version": 1, "lines": {
            "study:white": { "e2e4": card, "bad": card, "d2d4": { "ease": 9 } },
            "": { "e2e4": card },
            "list": [],
        }});
        assert_eq!(
            Value::Object(decode_schedule(&raw)),
            json!({ "study:white": { "e2e4": card } })
        );
    }

    #[test]
    fn recording_keeps_the_schedule_bounded() {
        let mut schedule = Map::new();
        record(&mut schedule, "s:white", "e2e4", true, NOW).expect("record");
        assert!(record(&mut schedule, "s:white", "nope", true, NOW).is_err());
        assert!(record(&mut schedule, "", "e2e4", true, NOW).is_err());
        let encoded = encode_schedule(schedule.clone());
        assert_eq!(decode_schedule(&encoded), schedule);

        // A full repertoire drops the line due last for a new one. The first line written is the
        // one due last, so a tie between every line (which picks the last written) would drop
        // another.
        let mut full = Map::new();
        let mut table = Map::new();
        let square = |i: usize| format!("{}{}", char::from(b'a' + (i % 8) as u8), 1 + i / 8);
        for i in 0..MAX_LINES {
            let card = Card {
                due: NOW + (MAX_LINES - i) as f64,
                ..Card::review(None, true, NOW)
            };
            table.insert(
                format!("{}{}", square(i % 64), square(i / 64)),
                card.to_value(),
            );
        }
        assert_eq!(table.len(), MAX_LINES);
        full.insert("k".into(), Value::Object(table));
        record(&mut full, "k", "h7h8q", false, NOW).expect("record");
        let table = full["k"].as_object().expect("table");
        assert_eq!(table.len(), MAX_LINES);
        assert!(table.contains_key("h7h8q"));
        assert!(!table.contains_key("a1a1"), "the line due last is dropped");
        assert!(table.contains_key("b1a1") && table.contains_key("h2h4"));

        let mut many = Map::new();
        for i in 0..MAX_KEYS {
            many.insert(format!("k{i}"), json!({}));
        }
        assert!(record(&mut many, "another", "e2e4", true, NOW).is_err());
        assert!(record(&mut many, "k0", "e2e4", true, NOW).is_ok());
    }

    #[test]
    fn a_repertoire_name_may_be_up_to_200_characters() {
        let longest = "é".repeat(MAX_KEY_CHARS);
        let too_long = "é".repeat(MAX_KEY_CHARS + 1);
        let mut schedule = Map::new();
        assert!(record(&mut schedule, &longest, "e2e4", true, NOW).is_ok());
        assert!(record(&mut schedule, &too_long, "e2e4", true, NOW).is_err());

        let card = Card::review(None, true, NOW).to_value();
        let raw = json!({ "version": 1, "lines": {
            (longest.clone()): { "e2e4": card },
            (too_long): { "e2e4": card },
        }});
        assert_eq!(
            Value::Object(decode_schedule(&raw)),
            json!({ (longest): { "e2e4": card } })
        );
    }

    #[test]
    fn counts_in_a_stored_card_are_whole_and_not_negative() {
        let card = Card::review(None, true, NOW).to_value();
        let with = |field: &str, value: Value| {
            let mut card = card.clone();
            card[field] = value;
            card
        };
        let raw = json!({ "version": 1, "lines": { "k": {
            "e2e4": card,
            "d2d4": with("reps", json!(1.5)),
            "c2c4": with("reps", json!(-1)),
            "g1f3": with("lapses", json!(-2.0)),
            "b1c3": with("lapses", json!(0.25)),
        }}});
        assert_eq!(
            Value::Object(decode_schedule(&raw)),
            json!({ "k": { "e2e4": card } })
        );
    }

    #[test]
    fn a_root_tag_is_exactly_8_hex_digits() {
        assert!(valid_line("0123abcd:e2e4"));
        // Hex digits, but not 8 of them.
        assert!(!valid_line("abc:e2e4"));
        assert!(!valid_line("0123abcd0:e2e4"));
        // 8 characters, but not hex digits.
        assert!(!valid_line("0123abcz:e2e4"));
    }

    #[test]
    fn due_lines_are_counted_and_the_most_overdue_comes_first() {
        let mut schedule = Map::new();
        record(&mut schedule, "k", "e2e4", false, NOW).expect("due now");
        record(&mut schedule, "k", "d2d4", false, NOW - 1000.0).expect("due earlier");
        record(&mut schedule, "k", "c2c4", true, NOW).expect("due tomorrow");
        let table = &schedule["k"];
        let lines = ["e2e4", "d2d4", "c2c4", "g1f3"];
        assert_eq!(
            summary(lines, table, NOW),
            Summary {
                due: 2,
                new: 1,
                next_due: Some(NOW + DAY_MS)
            }
        );
        assert_eq!(most_overdue(&lines, table, NOW), Some(1));
        assert_eq!(most_overdue(&["c2c4", "g1f3"], table, NOW), None);
        assert_eq!(summary(lines, &Value::Null, NOW).new, 4);
    }
}
