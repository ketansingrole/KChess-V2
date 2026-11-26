//! A game's headers and annotations in a few bytes, beside the columns and move code that hold
//! the rest. Broadcasts and online games put an engine eval and a clock after every move
//! (`{ [%eval 0.18] [%clk 1:30:58] }`), and several headers repeat one URL. Kept as text, that
//! made a game take about 5 KB; here a move's eval and clock take about four bytes, and a header
//! that starts like an earlier one stores only what follows the shared start.
//!
//! The format outlives the release that wrote it, so it only grows: a new header name or comment
//! shape is added at the end of its list, never in the middle. `pack` reads its result back and
//! compares it with the game, so a game it cannot hold exactly is left to its full text.
//!
//! Layout (numbers are LEB128; signed ones zigzag first; text is a length and UTF-8):
//! - the format, 1
//! - the headers: their number, then each one's name (0 and its text, or 1 + its index in
//!   `NAMES`) and value (0 the column's, 1 and its text, or 2 + an earlier header's index, the
//!   bytes shared with that header's value, and the text after them)
//! - the game's comments before its first move (a list, below)
//! - for each main-line move, a shape byte: bits 0–1 its comments (0 none, 1 one, whose shape is
//!   bits 4–7, 2 a list), bit 2 NAGs (their number, then each), bit 3 comments before the move (a
//!   list)
//! - a list is its length, then each comment as a shape byte and its values
//!
//! A comment's shape is 0 for text, or one of `SHAPES`: an eval is `zigzag(value) << 3 |
//! decimals << 1 | mate`, and a clock the change in seconds from the same side's last clock,
//! times four, plus its decimals (followed by their value when there are any).

use super::Stored;
use crate::pgn::{Game, PgnNode};

const FORMAT: u8 = super::EXTRA_NOTES;

/// Header names stored as a number; the kept headers first, so their values can be the columns'.
const NAMES: [&str; 37] = [
    "Event",
    "Site",
    "Date",
    "Round",
    "White",
    "Black",
    "Result",
    "WhiteElo",
    "BlackElo",
    "ECO",
    "Opening",
    "WhiteTitle",
    "BlackTitle",
    "WhiteFideId",
    "BlackFideId",
    "TimeControl",
    "Board",
    "Variant",
    "UTCDate",
    "UTCTime",
    "BroadcastName",
    "BroadcastURL",
    "GameURL",
    "Annotator",
    "PlyCount",
    "EventDate",
    "WhiteTeam",
    "BlackTeam",
    "FEN",
    "SetUp",
    "Termination",
    "WhiteRatingDiff",
    "BlackRatingDiff",
    "StudyName",
    "ChapterName",
    "ChapterURL",
    "Source",
];

/// The comments made only of an eval and a clock, in the order Lichess writes them and others.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    EvalClock = 1,
    Eval = 2,
    Clock = 3,
    ClockEval = 4,
}

const SHAPES: [Shape; 4] = [
    Shape::EvalClock,
    Shape::Eval,
    Shape::Clock,
    Shape::ClockEval,
];

/// The shortest shared start worth storing as a reference.
const SHARED_MIN: usize = 8;

struct Out(Vec<u8>);

impl Out {
    fn byte(&mut self, value: u8) {
        self.0.push(value);
    }

    fn uint(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.0.push((value as u8) | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    fn int(&mut self, value: i64) {
        self.uint(((value << 1) ^ (value >> 63)) as u64);
    }

    fn text(&mut self, text: &str) {
        self.uint(text.len() as u64);
        self.0.extend_from_slice(text.as_bytes());
    }
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&first, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(first)
    }

    fn uint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn int(&mut self) -> Option<i64> {
        let value = self.uint()?;
        Some(((value >> 1) as i64) ^ -((value & 1) as i64))
    }

    fn count(&mut self) -> Option<usize> {
        // No list is longer than the bytes left to hold it, so a damaged length cannot allocate.
        usize::try_from(self.uint()?)
            .ok()
            .filter(|&n| n <= self.0.len())
    }

    fn text(&mut self) -> Option<String> {
        let length = self.count()?;
        let (text, rest) = self.0.split_at_checked(length)?;
        self.0 = rest;
        String::from_utf8(text.to_vec()).ok()
    }
}

/// `value` and `decimals`: "-0.16" is (-16, 2), "#-3" a mate in 3 for Black.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Eval {
    value: i64,
    decimals: u8,
    mate: bool,
}

impl Eval {
    fn parse(text: &str) -> Option<Eval> {
        let eval = if let Some(mate) = text.strip_prefix('#') {
            Eval {
                value: mate.parse().ok()?,
                decimals: 0,
                mate: true,
            }
        } else {
            let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
            let decimals = u8::try_from(fraction.len()).ok().filter(|&d| d <= 3)?;
            let negative = whole.starts_with('-');
            let digits = format!("{}{fraction}", whole.trim_start_matches('-'));
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let magnitude: i64 = digits.parse().ok()?;
            Eval {
                value: if negative { -magnitude } else { magnitude },
                decimals,
                mate: false,
            }
        };
        // Only a value written back exactly is held as a number ("-0.0" and "+1" stay text).
        (eval.to_string() == text && eval.code().is_some()).then_some(eval)
    }

    fn code(self) -> Option<u64> {
        let value = (((self.value << 1) ^ (self.value >> 63)) as u64).checked_mul(8)?;
        Some(value | u64::from(self.decimals) << 1 | u64::from(self.mate))
    }

    fn from_code(code: u64) -> Eval {
        let value = code >> 3;
        Eval {
            value: ((value >> 1) as i64) ^ -((value & 1) as i64),
            decimals: ((code >> 1) & 3) as u8,
            mate: code & 1 == 1,
        }
    }
}

impl std::fmt::Display for Eval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.mate {
            return write!(f, "#{}", self.value);
        }
        let scale = 10i64.pow(u32::from(self.decimals));
        let sign = if self.value < 0 { "-" } else { "" };
        let magnitude = self.value.unsigned_abs();
        let whole = magnitude / scale.unsigned_abs();
        if self.decimals == 0 {
            write!(f, "{sign}{whole}")
        } else {
            let fraction = magnitude % scale.unsigned_abs();
            let width = usize::from(self.decimals);
            write!(f, "{sign}{whole}.{fraction:0width$}")
        }
    }
}

/// A clock as `H:MM:SS`, with up to three decimals of a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clock {
    seconds: i64,
    decimals: u8,
    fraction: u64,
}

impl Clock {
    fn parse(text: &str) -> Option<Clock> {
        let (time, fraction) = text.split_once('.').unwrap_or((text, ""));
        let mut parts = time.split(':');
        let hours: u32 = parts.next()?.parse().ok()?;
        let minutes: u32 = parts.next()?.parse().ok()?;
        let seconds: u32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let clock = Clock {
            seconds: i64::from(hours) * 3600 + i64::from(minutes) * 60 + i64::from(seconds),
            decimals: u8::try_from(fraction.len()).ok().filter(|&d| d <= 3)?,
            fraction: if fraction.is_empty() {
                0
            } else {
                fraction.parse().ok()?
            },
        };
        (clock.to_string() == text).then_some(clock)
    }
}

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.seconds;
        write!(f, "{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)?;
        if self.decimals > 0 {
            let width = usize::from(self.decimals);
            write!(f, ".{:0width$}", self.fraction)?;
        }
        Ok(())
    }
}

/// A command's value and what follows it: `[%clk 1:30:58] rest` gives ("1:30:58", " rest").
fn command<'a>(text: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    text.strip_prefix("[%")?
        .strip_prefix(name)?
        .strip_prefix(' ')?
        .split_once(']')
}

/// A comment's eval and clock when it holds nothing else, in a shape `Shape` names.
fn shape_of(comment: &str) -> Option<(Shape, Option<Eval>, Option<Clock>)> {
    if let Some((eval, rest)) = command(comment, "eval") {
        let eval = Eval::parse(eval)?;
        if rest.is_empty() {
            return Some((Shape::Eval, Some(eval), None));
        }
        let (clock, rest) = command(rest.strip_prefix(' ')?, "clk")?;
        return rest.is_empty().then_some((
            Shape::EvalClock,
            Some(eval),
            Some(Clock::parse(clock)?),
        ));
    }
    let (clock, rest) = command(comment, "clk")?;
    let clock = Clock::parse(clock)?;
    if rest.is_empty() {
        return Some((Shape::Clock, None, Some(clock)));
    }
    let (eval, rest) = command(rest.strip_prefix(' ')?, "eval")?;
    rest.is_empty()
        .then_some((Shape::ClockEval, Some(Eval::parse(eval)?), Some(clock)))
}

/// Writes and reads comments, each side's clock stored as its change from that side's last.
#[derive(Default)]
struct Comments {
    clocks: [i64; 2],
}

impl Comments {
    /// The comment's shape byte, and its values written to `values`.
    fn put(&mut self, comment: &str, side: usize, values: &mut Out) -> Option<u8> {
        let Some((shape, eval, clock)) = shape_of(comment) else {
            values.text(comment);
            return Some(0);
        };
        let put_eval = |values: &mut Out| -> Option<()> {
            values.uint(eval?.code()?);
            Some(())
        };
        let mut put_clock = |values: &mut Out| -> Option<()> {
            let clock = clock?;
            let last = self.clocks.get_mut(side)?;
            let change = clock.seconds.checked_sub(*last)?;
            *last = clock.seconds;
            values.int(change.checked_mul(4)? + i64::from(clock.decimals));
            if clock.decimals > 0 {
                values.uint(clock.fraction);
            }
            Some(())
        };
        match shape {
            Shape::EvalClock => {
                put_eval(values)?;
                put_clock(values)?;
            }
            Shape::Eval => put_eval(values)?,
            Shape::Clock => put_clock(values)?,
            Shape::ClockEval => {
                put_clock(values)?;
                put_eval(values)?;
            }
        }
        Some(shape as u8)
    }

    fn take(&mut self, shape: u8, side: usize, input: &mut In) -> Option<String> {
        if shape == 0 {
            return input.text();
        }
        let shape = *SHAPES.iter().find(|s| **s as u8 == shape)?;
        let eval = |input: &mut In| Some(Eval::from_code(input.uint()?));
        let mut clock = |input: &mut In| -> Option<Clock> {
            let code = input.int()?;
            let decimals = code.rem_euclid(4);
            let last = self.clocks.get_mut(side)?;
            *last = last.checked_add((code - decimals) / 4)?;
            Some(Clock {
                seconds: *last,
                decimals: decimals as u8,
                fraction: if decimals > 0 { input.uint()? } else { 0 },
            })
        };
        Some(match shape {
            Shape::EvalClock => {
                let eval = eval(input)?;
                format!("[%eval {eval}] [%clk {}]", clock(input)?)
            }
            Shape::Eval => format!("[%eval {}]", eval(input)?),
            Shape::Clock => format!("[%clk {}]", clock(input)?),
            Shape::ClockEval => {
                let clock = clock(input)?;
                format!("[%clk {clock}] [%eval {}]", eval(input)?)
            }
        })
    }

    fn put_list(&mut self, list: &[String], side: usize, out: &mut Out) -> Option<()> {
        out.uint(list.len() as u64);
        for comment in list {
            let mut values = Out(Vec::new());
            out.byte(self.put(comment, side, &mut values)?);
            out.0.append(&mut values.0);
        }
        Some(())
    }

    fn take_list(&mut self, side: usize, input: &mut In) -> Option<Vec<String>> {
        let count = input.count()?;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            let shape = input.byte()?;
            list.push(self.take(shape, side, input)?);
        }
        Some(list)
    }
}

/// The value a kept header is written with from its column; None when it is not written.
pub(super) fn column(stored: &Stored, name: &str) -> Option<String> {
    let or_unknown = |value: &Option<String>| value.clone().unwrap_or_else(|| "?".to_string());
    let player = |name: &str| {
        if name.is_empty() {
            "?".to_string()
        } else {
            name.to_string()
        }
    };
    Some(match name {
        "Event" => or_unknown(&stored.event),
        "Site" => or_unknown(&stored.site),
        "Date" => stored
            .date
            .clone()
            .unwrap_or_else(|| "????.??.??".to_string()),
        "Round" => or_unknown(&stored.round),
        "White" => player(&stored.white),
        "Black" => player(&stored.black),
        "Result" => stored.outcome.header().to_string(),
        "WhiteElo" => stored.white_elo?.to_string(),
        "BlackElo" => stored.black_elo?.to_string(),
        "ECO" => stored.eco.clone()?,
        "Opening" => stored.opening.clone()?,
        _ => return None,
    })
}

/// The bytes `a` and `b` start with alike, ending on a character boundary.
fn shared(a: &str, b: &str) -> usize {
    let mut length = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    while !b.is_char_boundary(length) {
        length -= 1;
    }
    length
}

fn put_headers(game: &Game, stored: &Stored, out: &mut Out) {
    out.uint(game.headers.len() as u64);
    for (index, (name, value)) in game.headers.iter().enumerate() {
        match NAMES.iter().position(|known| known == name) {
            Some(at) => out.uint(at as u64 + 1),
            None => {
                out.uint(0);
                out.text(name);
            }
        }
        if column(stored, name).as_deref() == Some(value.as_str()) {
            out.uint(0);
            continue;
        }
        let earlier = game.headers.iter().take(index).enumerate();
        let best = earlier
            .map(|(at, (_, other))| (shared(other, value), at))
            .max_by_key(|&(length, at)| (length, std::cmp::Reverse(at)));
        match best {
            Some((length, at)) if length >= SHARED_MIN => {
                out.uint(at as u64 + 2);
                out.uint(length as u64);
                out.text(value.get(length..).unwrap_or_default());
            }
            _ => {
                out.uint(1);
                out.text(value);
            }
        }
    }
}

fn take_headers(stored: &Stored, input: &mut In) -> Option<Vec<(String, String)>> {
    let count = input.count()?;
    let mut headers: Vec<(String, String)> = Vec::with_capacity(count);
    for _ in 0..count {
        let name = match input.uint()? {
            0 => input.text()?,
            at => NAMES.get(usize::try_from(at - 1).ok()?)?.to_string(),
        };
        let value = match input.uint()? {
            0 => column(stored, &name)?,
            1 => input.text()?,
            at => {
                let (_, other) = headers.get(usize::try_from(at - 2).ok()?)?;
                let length = usize::try_from(input.uint()?).ok()?;
                format!("{}{}", other.get(..length)?, input.text()?)
            }
        };
        headers.push((name, value));
    }
    Some(headers)
}

/// The value of the header `name` (`TimeControl`, say), read from packed notes without unpacking
/// the game; None when it has none. A header whose value is a column's is read from `stored`'s
/// columns: without them, it and a header written as sharing its start with it are unknown.
pub(super) fn header(notes: &[u8], name: &str, stored: Option<&Stored>) -> Option<String> {
    let mut input = In(notes);
    if input.byte()? != FORMAT {
        return None;
    }
    let count = input.count()?;
    let mut values: Vec<Option<String>> = Vec::with_capacity(count);
    for _ in 0..count {
        let header = match input.uint()? {
            0 => input.text()?,
            at => NAMES.get(usize::try_from(at - 1).ok()?)?.to_string(),
        };
        let value = match input.uint()? {
            0 => stored.and_then(|stored| column(stored, &header)),
            1 => Some(input.text()?),
            at => {
                let other = values.get(usize::try_from(at - 2).ok()?)?.clone();
                let length = usize::try_from(input.uint()?).ok()?;
                // Read whether or not it is known, so the headers after it are: a Lichess game's
                // `UTCDate` shares its start with its `Date`, a column, and hid its
                // `TimeControl` when this stopped at it.
                let rest = input.text()?;
                other.and_then(|other| Some(format!("{}{rest}", other.get(..length)?)))
            }
        };
        if header == name {
            return value;
        }
        values.push(value);
    }
    None
}

fn option_list(list: &Option<Vec<String>>) -> &[String] {
    list.as_deref().unwrap_or_default()
}

/// The side that played the `index`th main-line move: 0 White's first move's slot, 1 the next.
fn side(index: usize) -> usize {
    index % 2
}

/// `game`'s headers and annotations, for a game whose moves are `stored`'s: None when it has
/// variations (only the main line is in the move code) or would not read back exactly.
pub(super) fn pack(game: &Game, stored: &Stored, sans: &[String]) -> Option<Vec<u8>> {
    let line: Vec<usize> = game.main_line().collect();
    if game.nodes.len() != line.len() + 1 || line.len() != sans.len() {
        return None;
    }
    let mut out = Out(vec![FORMAT]);
    put_headers(game, stored, &mut out);
    let mut comments = Comments::default();
    put_option_list(&mut comments, &game.comments, 0, &mut out)?;
    for (index, &id) in line.iter().enumerate() {
        let node = game.node(id);
        let side = side(index);
        let mut values = Out(Vec::new());
        let mut shape = 0u8;
        if let Some(starting) = &node.starting_comments {
            shape |= 8;
            comments.put_list(starting, side, &mut values)?;
        }
        if let Some(nags) = &node.nags {
            shape |= 4;
            values.uint(nags.len() as u64);
            for &nag in nags {
                values.uint(u64::from(nag));
            }
        }
        match node.comments.as_deref() {
            None => {}
            Some([one]) => {
                let mut value = Out(Vec::new());
                let kind = comments.put(one, side, &mut value)?;
                shape |= 1 | kind << 4;
                values.0.append(&mut value.0);
            }
            Some(list) => {
                shape |= 2;
                comments.put_list(list, side, &mut values)?;
            }
        }
        out.byte(shape);
        out.0.append(&mut values.0);
    }
    let packed = out.0;
    let back = unpack(&packed, stored, sans)?;
    same(game, &back).then_some(packed)
}

fn put_option_list(
    comments: &mut Comments,
    list: &Option<Vec<String>>,
    side: usize,
    out: &mut Out,
) -> Option<()> {
    match list {
        None => out.byte(0),
        Some(list) => {
            out.byte(1);
            comments.put_list(list, side, out)?;
        }
    }
    Some(())
}

/// The game `pack` packed, its main-line moves written as `sans`.
pub(super) fn unpack(notes: &[u8], stored: &Stored, sans: &[String]) -> Option<Game> {
    let mut input = In(notes);
    if input.byte()? != FORMAT {
        return None;
    }
    let mut game = Game::empty();
    game.headers = take_headers(stored, &mut input)?;
    let mut comments = Comments::default();
    game.comments = match input.byte()? {
        0 => None,
        1 => Some(comments.take_list(0, &mut input)?),
        _ => return None,
    };
    let mut parent = 0;
    for (index, san) in sans.iter().enumerate() {
        let side = side(index);
        let shape = input.byte()?;
        let starting = if shape & 8 != 0 {
            Some(comments.take_list(side, &mut input)?)
        } else {
            None
        };
        let nags = if shape & 4 != 0 {
            let count = input.count()?;
            let mut nags = Vec::with_capacity(count);
            for _ in 0..count {
                nags.push(u32::try_from(input.uint()?).ok()?);
            }
            Some(nags)
        } else {
            None
        };
        let node_comments = match shape & 3 {
            0 => None,
            1 => Some(vec![comments.take(shape >> 4, side, &mut input)?]),
            2 => Some(comments.take_list(side, &mut input)?),
            _ => return None,
        };
        parent = game.add_move(parent, PgnNode::with(san, node_comments, starting, nags));
    }
    input.0.is_empty().then_some(game)
}

/// Whether `back` writes the same PGN as `game` with its main line in standard SAN: the same
/// headers, comments and NAGs, in the same places.
fn same(game: &Game, back: &Game) -> bool {
    let annotations = |game: &Game| -> Vec<(Vec<String>, Vec<String>, Vec<u32>)> {
        game.main_line()
            .map(|id| {
                let node = game.node(id);
                (
                    option_list(&node.comments).to_vec(),
                    option_list(&node.starting_comments).to_vec(),
                    node.nags.clone().unwrap_or_default(),
                )
            })
            .collect()
    };
    let presence = |game: &Game| -> Vec<(bool, bool, bool)> {
        game.main_line()
            .map(|id| {
                let node = game.node(id);
                (
                    node.comments.is_some(),
                    node.starting_comments.is_some(),
                    node.nags.is_some(),
                )
            })
            .collect()
    };
    game.headers == back.headers
        && game.comments == back.comments
        && annotations(game) == annotations(back)
        && presence(game) == presence(back)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evals_and_clocks_read_back_only_when_written_back_exactly() {
        for text in ["0.18", "-0.16", "0.0", "10", "#-3", "#5", "-12.345"] {
            let eval = Eval::parse(text).unwrap();
            assert_eq!(Eval::from_code(eval.code().unwrap()), eval, "{text}");
            assert_eq!(eval.to_string(), text);
        }
        for text in ["-0.0", "+1", "0.1234", "", "-", "1e3", ".5", "#"] {
            assert_eq!(Eval::parse(text), None, "{text}");
        }
        for text in ["1:30:58", "0:00:05.3", "12:00:00", "0:00:00.125"] {
            assert_eq!(Clock::parse(text).unwrap().to_string(), text);
        }
        for text in [
            "01:30:58",
            "1:3:58",
            "90:58",
            "1:30:58:00",
            "1:30:58.",
            "1:30:5a",
        ] {
            assert_eq!(Clock::parse(text), None, "{text}");
        }
    }

    #[test]
    fn numbers_round_trip() {
        let mut out = Out(Vec::new());
        let values = [0i64, 1, -1, 63, -64, 8191, -8192, i64::MAX, i64::MIN];
        for value in values {
            out.int(value);
        }
        let mut input = In(&out.0);
        for value in values {
            assert_eq!(input.int(), Some(value));
        }
        assert_eq!(input.int(), None);
    }

    #[test]
    fn a_damaged_list_length_is_refused() {
        let mut out = Out(Vec::new());
        out.uint(u64::MAX);
        assert_eq!(In(&out.0).count(), None);
        assert_eq!(In(&[0x80, 0x80]).uint(), None);
    }
}
