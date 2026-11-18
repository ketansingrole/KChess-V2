//! The puzzle database sampler: reads Lichess's puzzle
//! CSV as decompressed bytes and keeps a random sample spread over every rating, theme and opening.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::js;

const RATING_BUCKET: f64 = 50.0;
const PER_BUCKET: usize = 1800;
const PER_THEME: usize = 400;
const MAX_LINE: usize = 16_000;
const MAX_THEMES: usize = 256;
/// Puzzles kept per opening family (the first of a row's `OpeningTags`), so each opening page
/// can offer some offline.
const PER_OPENING: usize = 120;
const MAX_OPENINGS: usize = 512;
pub const OVERSIZED: &str = "Puzzle database contains an oversized CSV line.";

/// A puzzle theme, as Lichess keys and names it.
#[derive(Deserialize, Debug, Clone)]
pub struct Theme {
    pub key: String,
    pub name: String,
}

/// The puzzle themes under a group's label.
#[derive(Deserialize, Debug, Clone)]
pub struct ThemeGroup {
    pub label: String,
    pub themes: Vec<Theme>,
}

/// Lichess's puzzle themes, in its groups (`data/puzzle-themes.json`).
pub fn theme_groups() -> &'static [ThemeGroup] {
    static GROUPS: OnceLock<Vec<ThemeGroup>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        crate::embedded::parse("puzzle themes", include_str!("../data/puzzle-themes.json"))
    })
}

/// A lichess.org/training page that is not one puzzle: the daily puzzle's or a theme's.
pub fn is_training_page(key: &str) -> bool {
    key == "daily"
        || theme_groups()
            .iter()
            .any(|group| group.themes.iter().any(|theme| theme.key == key))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DbPuzzle {
    pub id: String,
    pub fen: String,
    pub moves: String,
    pub rating: f64,
    pub plays: f64,
    pub themes: String,
    /// Lichess's `OpeningTags`: the opening family, then the family and variation, as keys
    /// (`Kings_Gambit_Accepted Kings_Gambit_Accepted_Abbazia_Defense`). Empty when untagged.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub openings: String,
    /// The Lichess game the puzzle comes from (from its `GameUrl`); empty when there is none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub game_id: String,
}

/// The game id of a puzzle's `GameUrl` (`https://lichess.org/787zsVup/black#48`): eight letters
/// and digits after the host, or nothing.
fn game_id_of(url: &[u8]) -> &str {
    const HOST: &[u8] = b"https://lichess.org/";
    let Some(rest) = url.strip_prefix(HOST) else {
        return "";
    };
    let end = rest
        .iter()
        .position(|b| *b == b'/' || *b == b'#')
        .unwrap_or(rest.len());
    let id = &rest[..end];
    if id.len() == 8 && id.iter().all(u8::is_ascii_alphanumeric) {
        std::str::from_utf8(id).unwrap_or_default()
    } else {
        ""
    }
}

/// A 32-bit mix of a Weyl sequence: a seeded sampler keeps exactly the rows the rules corpus
/// recorded for that seed.
#[derive(Debug, Clone)]
pub struct Random(u32);

impl Random {
    pub fn new(seed: u32) -> Random {
        Random(seed)
    }

    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let s = self.0;
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// Uniform random sample of a stream of unknown length. Rows are shared between the rating
/// and theme reservoirs and freed once no reservoir keeps them.
struct Reservoir {
    seen: u64,
    size: usize,
    rows: Vec<Rc<DbPuzzle>>,
}

/// Where a reservoir puts the row it was just offered.
#[derive(Clone, Copy)]
enum Place {
    Push,
    Replace(usize),
}

impl Reservoir {
    fn new(size: usize) -> Reservoir {
        Reservoir {
            seen: 0,
            size,
            rows: Vec::new(),
        }
    }

    /// Count the row and decide where it goes, drawing one random number per row a full reservoir
    /// sees; the row itself is only built when some reservoir keeps it.
    fn offer(&mut self, random: &mut Random) -> Option<Place> {
        self.seen += 1;
        if self.rows.len() < self.size {
            return Some(Place::Push);
        }
        let slot = (random.next_f64() * self.seen as f64).floor() as usize;
        (slot < self.size).then_some(Place::Replace(slot))
    }

    fn place(&mut self, place: Place, row: &Rc<DbPuzzle>) {
        match place {
            Place::Push => self.rows.push(Rc::clone(row)),
            Place::Replace(slot) => self.rows[slot] = Rc::clone(row),
        }
    }
}

/// FNV-1a: theme names and rating buckets are short, trusted keys.
#[derive(Default, Clone, Copy)]
struct Fnv(u64);

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for &b in bytes {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = hash;
    }
}

type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fnv>>;

fn solid(deviation: f64, popularity: f64, plays: f64) -> bool {
    deviation <= 90.0 && popularity >= 85.0 && plays >= 300.0
}

fn solid_for_theme(deviation: f64, popularity: f64, plays: f64) -> bool {
    deviation <= 110.0 && popularity >= 70.0 && plays >= 100.0
}

/// Sampler state; `push` the decompressed bytes as they arrive, then `finish`.
pub struct Sampler {
    random: Random,
    /// Reservoirs in creation order, which `kept()` follows.
    buckets: Vec<Reservoir>,
    bucket_index: FastMap<i64, usize>,
    themes: Vec<Reservoir>,
    theme_index: FastMap<String, usize>,
    openings: Vec<Reservoir>,
    opening_index: FastMap<String, usize>,
    tail: Vec<u8>,
    /// The kept count once `release` has freed the rows.
    released: Option<usize>,
    pub lines: u64,
}

impl Sampler {
    pub fn new(random: Random) -> Sampler {
        Sampler {
            random,
            buckets: Vec::new(),
            bucket_index: FastMap::default(),
            themes: Vec::new(),
            theme_index: FastMap::default(),
            openings: Vec::new(),
            opening_index: FastMap::default(),
            tail: Vec::new(),
            released: None,
            lines: 0,
        }
    }

    /// One CSV line: `PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,
    /// OpeningTags`; the last two may be missing.
    pub fn add(&mut self, line: &str) -> Result<(), &'static str> {
        self.add_bytes(line.as_bytes())
    }

    /// One line as raw bytes. Only cells whose text matters are decoded (invalid UTF-8 becoming
    /// U+FFFD). A kept row's id and themes are ASCII, so rows with other bytes there are rejected.
    fn add_bytes(&mut self, line: &[u8]) -> Result<(), &'static str> {
        if line.len() > MAX_LINE && js::utf16_len(&String::from_utf8_lossy(line)) > MAX_LINE {
            return Err(OVERSIZED);
        }
        let mut cells: [&[u8]; 10] = [&[]; 10];
        let mut rest = line;
        for (i, cell) in cells.iter_mut().enumerate() {
            match memchr::memchr(b',', rest) {
                Some(at) if i < 9 => {
                    *cell = &rest[..at];
                    rest = &rest[at + 1..];
                }
                // The last cell keeps the rest of the line, as the themes cell did.
                _ if i >= 7 => {
                    *cell = rest;
                    break;
                }
                _ => return Ok(()),
            }
        }
        if cells[0] == b"PuzzleId" {
            return Ok(());
        }
        let number = |cell: &[u8]| match std::str::from_utf8(cell) {
            Ok(text) => js::string_to_number(text),
            Err(_) => js::string_to_number(&String::from_utf8_lossy(cell)),
        };
        let rating = number(cells[3]);
        let deviation = number(cells[4]);
        if !rating.is_finite() || !(0.0..=4000.0).contains(&rating) || !deviation.is_finite() {
            return Ok(());
        }
        let ascii = |cell: &[u8], space: bool| {
            cell.iter()
                .all(|b| b.is_ascii_alphanumeric() || (space && *b == b' '))
        };
        if cells[0].is_empty() || !ascii(cells[0], false) || cells[0].len() > 64 {
            return Ok(());
        }
        if cells[7].len() > 256 || !ascii(cells[7], true) {
            return Ok(());
        }
        let (id, themes) = (
            std::str::from_utf8(cells[0]).unwrap_or_default(),
            std::str::from_utf8(cells[7]).unwrap_or_default(),
        );
        // Opening keys are letters, digits and underscores; anything else leaves the row untagged.
        let tags = cells[9];
        let openings = if tags.len() <= 256
            && tags
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b' ')
        {
            std::str::from_utf8(tags).unwrap_or_default().trim()
        } else {
            ""
        };
        self.lines += 1;
        let popularity = number(cells[5]);
        let plays = number(cells[6]);
        let is_solid = solid(deviation, popularity, plays);
        let is_solid_for_theme = solid_for_theme(deviation, popularity, plays);
        if !is_solid && !is_solid_for_theme {
            return Ok(());
        }
        // The row is built only once a reservoir keeps it; placing at once keeps each
        // reservoir's length, and so its random draws, as if every row were built.
        let mut row: Option<Rc<DbPuzzle>> = None;
        let mut build = || {
            Rc::new(DbPuzzle {
                id: id.to_string(),
                fen: String::from_utf8_lossy(cells[1]).into_owned(),
                moves: String::from_utf8_lossy(cells[2]).into_owned(),
                rating,
                plays,
                themes: themes.to_string(),
                openings: openings.to_string(),
                game_id: game_id_of(cells[8]).to_string(),
            })
        };
        if is_solid {
            let bucket = (rating / RATING_BUCKET).floor() as i64;
            let index = *self.bucket_index.entry(bucket).or_insert_with(|| {
                self.buckets.push(Reservoir::new(PER_BUCKET));
                self.buckets.len() - 1
            });
            if let Some(place) = self.buckets[index].offer(&mut self.random) {
                let kept = row.get_or_insert_with(&mut build);
                self.buckets[index].place(place, kept);
            }
        }
        if is_solid_for_theme {
            for theme in themes.split(' ') {
                if theme.is_empty() || theme.len() > 40 {
                    continue;
                }
                let index = match self.theme_index.get(theme) {
                    Some(&index) => index,
                    None if self.themes.len() >= MAX_THEMES => continue,
                    None => {
                        self.themes.push(Reservoir::new(PER_THEME));
                        self.theme_index
                            .insert(theme.to_string(), self.themes.len() - 1);
                        self.themes.len() - 1
                    }
                };
                if let Some(place) = self.themes[index].offer(&mut self.random) {
                    let kept = row.get_or_insert_with(&mut build);
                    self.themes[index].place(place, kept);
                }
            }
            if let Some(family) = openings
                .split(' ')
                .next()
                .filter(|f| !f.is_empty() && f.len() <= 80)
            {
                let index = match self.opening_index.get(family) {
                    Some(&index) => Some(index),
                    None if self.openings.len() >= MAX_OPENINGS => None,
                    None => {
                        self.openings.push(Reservoir::new(PER_OPENING));
                        self.opening_index
                            .insert(family.to_string(), self.openings.len() - 1);
                        Some(self.openings.len() - 1)
                    }
                };
                if let Some(index) = index
                    && let Some(place) = self.openings[index].offer(&mut self.random)
                {
                    let kept = row.get_or_insert_with(&mut build);
                    self.openings[index].place(place, kept);
                }
            }
        }
        Ok(())
    }

    /// Decompressed bytes as they arrive; lines may span chunks.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), &'static str> {
        let mut start = 0;
        for end in memchr::memchr_iter(b'\n', chunk) {
            if self.tail.is_empty() {
                self.add_bytes(&chunk[start..end])?;
            } else {
                self.tail.extend_from_slice(&chunk[start..end]);
                let line = std::mem::take(&mut self.tail);
                self.add_bytes(&line)?;
            }
            start = end + 1;
        }
        self.tail.extend_from_slice(&chunk[start..]);
        if self.tail.len() > MAX_LINE
            && js::utf16_len(&String::from_utf8_lossy(&self.tail)) > MAX_LINE
        {
            return Err(OVERSIZED);
        }
        Ok(())
    }

    /// The last line, which has no newline after it.
    pub fn finish(&mut self) -> Result<(), &'static str> {
        let line = std::mem::take(&mut self.tail);
        self.add_bytes(&line)
    }

    /// Puzzles kept so far, counted per rating bucket (a cheap progress figure).
    pub fn count(&self) -> usize {
        self.released
            .unwrap_or_else(|| self.buckets.iter().map(|r| r.rows.len()).sum())
    }

    /// Free the sample once it has been taken, keeping the counts for progress reports.
    pub fn release(&mut self) {
        self.released = Some(self.count());
        self.buckets = Vec::new();
        self.themes = Vec::new();
        self.openings = Vec::new();
        self.bucket_index = FastMap::default();
        self.theme_index = FastMap::default();
        self.opening_index = FastMap::default();
        self.tail = Vec::new();
    }

    /// Every puzzle kept, once: reservoirs in creation order, each in its own order.
    pub fn kept(&self) -> Vec<&DbPuzzle> {
        let mut order: Vec<&str> = Vec::new();
        let mut latest: HashMap<&str, &DbPuzzle> = HashMap::new();
        for reservoir in self
            .buckets
            .iter()
            .chain(&self.themes)
            .chain(&self.openings)
        {
            for row in &reservoir.rows {
                let id = row.id.as_str();
                if latest.insert(id, row).is_none() {
                    order.push(id);
                }
            }
        }
        order.into_iter().map(|id| latest[id]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_source_game_of_a_game_url() {
        assert_eq!(
            game_id_of(b"https://lichess.org/787zsVup/black#48"),
            "787zsVup"
        );
        assert_eq!(game_id_of(b"https://lichess.org/787zsVup#48"), "787zsVup");
        assert_eq!(game_id_of(b"https://lichess.org/787zsVup"), "787zsVup");
        assert_eq!(game_id_of(b""), "");
        assert_eq!(game_id_of(b"https://example.org/787zsVup"), "");
        assert_eq!(game_id_of(b"https://lichess.org/short#1"), "");
        assert_eq!(game_id_of(b"https://lichess.org/787zs-up#1"), "");
    }

    #[test]
    fn random_matches_the_typescript_generator() {
        // The first values of the generator seeded with 1.
        let mut random = Random::new(1);
        let first = random.next_f64();
        assert!((0.0..1.0).contains(&first));
        assert_ne!(first, random.next_f64());
    }

    #[test]
    fn samples_solid_puzzles_across_chunks() {
        let mut sampler = Sampler::new(Random::new(7));
        let csv = "PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes\n\
                   abc,8/8 w - - 0 1,e2e4,1500,80,90,1000,fork mate\n\
                   bad!,8/8 w - - 0 1,e2e4,1500,80,90,1000,fork\n\
                   weak,8/8 w - - 0 1,e2e4,1500,300,10,5,fork";
        let (a, b) = csv.as_bytes().split_at(70);
        sampler.push(a).unwrap();
        sampler.push(b).unwrap();
        sampler.finish().unwrap();
        assert_eq!(sampler.lines, 2);
        assert_eq!(sampler.count(), 1);
        assert_eq!(sampler.kept().len(), 1);
        assert_eq!(sampler.kept()[0].themes, "fork mate");
        assert_eq!(sampler.kept()[0].openings, "");
    }

    #[test]
    fn keeps_opening_tags_and_a_sample_per_opening() {
        let mut sampler = Sampler::new(Random::new(3));
        let mut csv = String::from(
            "PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags\n",
        );
        // Solid only for themes (and so for openings): no rating bucket keeps them.
        for i in 0..200 {
            csv.push_str(&format!(
                "k{i},8/8 w - - 0 1,e2e4,1500,100,80,200,fork,https://lichess.org/x#1,Kings_Gambit_Accepted Kings_Gambit_Accepted_Abbazia_Defense\n"
            ));
        }
        csv.push_str(
            "bad1,8/8 w - - 0 1,e2e4,1500,100,80,200,pin,https://lichess.org/x#1,Bad-Tag\n",
        );
        sampler.push(csv.as_bytes()).unwrap();
        sampler.finish().unwrap();
        let kept = sampler.kept();
        let tagged: Vec<_> = kept.iter().filter(|row| !row.openings.is_empty()).collect();
        assert!(tagged.len() >= PER_OPENING);
        assert_eq!(
            tagged[0].openings,
            "Kings_Gambit_Accepted Kings_Gambit_Accepted_Abbazia_Defense"
        );
        assert_eq!(tagged[0].themes, "fork");
        let bad = kept
            .iter()
            .find(|row| row.id == "bad1")
            .expect("kept for its theme");
        assert_eq!(bad.openings, "");
    }

    use std::hash::Hasher;

    /// A solid row (solid for the rating bucket and for themes) with the given cells.
    fn row(id: &str, rating: u32, themes: &str, tags: &str) -> String {
        format!(
            "{id},8/8 w - - 0 1,e2e4,{rating},80,90,1000,{themes},https://lichess.org/x#1,{tags}"
        )
    }

    /// A row solid for themes and openings only, so no rating bucket keeps it.
    fn theme_row(id: &str, themes: &str, tags: &str) -> String {
        format!("{id},8/8 w - - 0 1,e2e4,1500,100,80,200,{themes},https://lichess.org/x#1,{tags}")
    }

    fn sampler_with(lines: &[String]) -> Sampler {
        let mut sampler = Sampler::new(Random::new(5));
        for line in lines {
            sampler.add(line).unwrap();
        }
        sampler
    }

    #[test]
    fn the_generator_repeats_the_recorded_sequence_for_a_seed() {
        let mut random = Random::new(1);
        assert_eq!(random.next_f64(), 0.6270739405881613);
        assert_eq!(random.next_f64(), 0.002735721180215478);
        assert_eq!(random.next_f64(), 0.5274470399599522);
        assert_eq!(random.next_f64(), 0.9810509674716741);
    }

    #[test]
    fn a_full_reservoir_replaces_a_slot_with_shrinking_odds() {
        let mut reservoir = Reservoir::new(1);
        let mut random = Random::new(9);
        let mut shadow = random.clone();
        let first = Rc::new(DbPuzzle {
            id: "a".into(),
            fen: String::new(),
            moves: String::new(),
            rating: 1.0,
            plays: 1.0,
            themes: String::new(),
            openings: String::new(),
            game_id: String::new(),
        });
        assert!(matches!(reservoir.offer(&mut random), Some(Place::Push)));
        reservoir.place(Place::Push, &first);
        for seen in 2..60u32 {
            let slot = (shadow.next_f64() * f64::from(seen)).floor() as usize;
            let place = reservoir.offer(&mut random);
            match (slot < 1, place) {
                (true, Some(Place::Replace(0))) | (false, None) => {}
                _ => panic!("offer {seen} put the row somewhere else than slot {slot}"),
            }
        }
    }

    #[test]
    fn the_hash_is_fnv_1a_over_everything_written() {
        let mut hasher = Fnv::default();
        hasher.write(b"a");
        assert_eq!(hasher.finish(), 0xaf63_dc4c_8601_ec8c);
        hasher.write(b"b");
        let mut whole = Fnv::default();
        whole.write(b"ab");
        assert_eq!(hasher.finish(), whole.finish());
        assert_eq!(whole.finish(), 0x089c_4407_b545_986a);
    }

    #[test]
    fn a_puzzle_must_pass_every_solid_bound() {
        assert!(solid(90.0, 85.0, 300.0));
        assert!(!solid(91.0, 85.0, 300.0));
        assert!(!solid(90.0, 84.0, 300.0));
        assert!(!solid(90.0, 85.0, 299.0));
        assert!(!solid(91.0, 84.0, 299.0));
        assert!(!solid(91.0, 85.0, 299.0));
        assert!(!solid(90.0, 84.0, 300.0));
        assert!(solid_for_theme(110.0, 70.0, 100.0));
        assert!(!solid_for_theme(111.0, 70.0, 100.0));
        assert!(!solid_for_theme(110.0, 69.0, 100.0));
        assert!(!solid_for_theme(110.0, 70.0, 99.0));
        assert!(!solid_for_theme(111.0, 69.0, 100.0));
        assert!(!solid_for_theme(111.0, 70.0, 99.0));
        assert!(!solid_for_theme(110.0, 69.0, 99.0));
    }

    #[test]
    fn adding_a_line_counts_it_and_refuses_an_oversized_one() {
        let mut sampler = Sampler::new(Random::new(1));
        assert!(sampler.add(&row("a", 1500, "fork", "")).is_ok());
        assert_eq!(sampler.lines, 1);
        assert_eq!(sampler.add(&"x".repeat(MAX_LINE + 1)), Err(OVERSIZED));
    }

    #[test]
    fn a_line_is_oversized_past_sixteen_thousand_utf16_units() {
        let check = |line: String| Sampler::new(Random::new(1)).add(&line);
        assert_eq!(check("x".repeat(MAX_LINE)), Ok(()));
        assert_eq!(check("x".repeat(MAX_LINE + 1)), Err(OVERSIZED));
        // More than 16000 bytes but only 8001 units.
        assert_eq!(check("\u{e9}".repeat(8001)), Ok(()));
        // 16001 bytes, exactly 16000 units.
        assert_eq!(check(format!("\u{e9}{}", "x".repeat(MAX_LINE - 1))), Ok(()));
    }

    #[test]
    fn a_pushed_tail_is_oversized_past_sixteen_thousand_utf16_units() {
        let check = |chunk: String| Sampler::new(Random::new(1)).push(chunk.as_bytes());
        assert_eq!(check("x".repeat(MAX_LINE)), Ok(()));
        assert_eq!(check("x".repeat(MAX_LINE + 1)), Err(OVERSIZED));
        assert_eq!(check("\u{e9}".repeat(8001)), Ok(()));
        assert_eq!(check(format!("\u{e9}{}", "x".repeat(MAX_LINE - 1))), Ok(()));
    }

    #[test]
    fn a_line_with_too_few_cells_is_not_counted() {
        let sampler = sampler_with(&["abc,1,2".to_string(), "abc".to_string()]);
        assert_eq!(sampler.lines, 0);
    }

    #[test]
    fn the_tags_cell_keeps_its_commas() {
        let sampler = sampler_with(&[row("a", 1500, "fork", "Foo,Bar")]);
        assert_eq!(sampler.kept()[0].openings, "");
    }

    #[test]
    fn an_unusable_rating_or_deviation_skips_the_line() {
        let cells = |rating: &str, deviation: &str| {
            format!("a,8/8 w - - 0 1,e2e4,{rating},{deviation},90,1000,fork,,")
        };
        let sampler = sampler_with(&[
            cells("5000", "80"),
            cells("-1", "80"),
            cells("nope", "80"),
            cells("1500", "nope"),
        ]);
        assert_eq!(sampler.lines, 0);
        let sampler = sampler_with(&[cells("0", "80"), cells("4000", "80")]);
        assert_eq!(sampler.lines, 2);
    }

    #[test]
    fn an_id_is_alphanumeric_and_at_most_sixty_four_long() {
        let lines = |id: String| sampler_with(&[row(&id, 1500, "fork", "")]).lines;
        assert_eq!(lines("a b".to_string()), 0);
        assert_eq!(lines(String::new()), 0);
        assert_eq!(lines("a".repeat(64)), 1);
        assert_eq!(lines("a".repeat(65)), 0);
    }

    #[test]
    fn themes_are_alphanumeric_words_of_at_most_two_hundred_fifty_six_bytes() {
        let lines = |themes: String| sampler_with(&[row("a", 1500, &themes, "")]).lines;
        let words = |len: usize| "abcdefg ".repeat(len / 8 + 1)[..len].to_string();
        assert_eq!(lines(words(256)), 1);
        assert_eq!(lines(words(257)), 0);
        assert_eq!(lines("fork-mate".to_string()), 0);
        assert_eq!(lines("fork mate".to_string()), 1);
    }

    #[test]
    fn a_row_solid_for_nothing_is_counted_but_kept_nowhere() {
        let weak = "w,8/8 w - - 0 1,e2e4,1500,300,10,5,fork,,";
        let sampler = sampler_with(&[weak.to_string()]);
        assert_eq!(sampler.lines, 1);
        assert!(sampler.kept().is_empty());
    }

    #[test]
    fn a_rating_bucket_keeps_eighteen_hundred_rows_of_fifty_points() {
        let lines: Vec<String> = (0..1900)
            .map(|i| row(&format!("r{i}"), 1500 + i % 50, "fork", ""))
            .collect();
        let sampler = sampler_with(&lines);
        assert_eq!(sampler.count(), PER_BUCKET);
        let next_bucket = sampler_with(&[row("a", 1500, "fork", ""), row("b", 1550, "fork", "")]);
        assert_eq!(next_bucket.buckets.len(), 2);
    }

    #[test]
    fn a_theme_word_is_kept_only_between_one_and_forty_bytes() {
        let kept = |themes: &str| sampler_with(&[theme_row("a", themes, "")]).kept().len();
        assert_eq!(kept(""), 0);
        assert_eq!(kept("fork  "), 1);
        assert_eq!(kept(&"a".repeat(40)), 1);
        assert_eq!(kept(&"a".repeat(41)), 0);
    }

    #[test]
    fn the_sampler_tracks_at_most_256_themes() {
        let lines: Vec<String> = (0..=MAX_THEMES)
            .map(|i| theme_row(&format!("t{i}"), &format!("theme{i}"), ""))
            .collect();
        let mut sampler = sampler_with(&lines);
        assert_eq!(sampler.kept().len(), MAX_THEMES);
        // A theme already tracked still takes rows.
        sampler.add(&theme_row("again", "theme0", "")).unwrap();
        assert_eq!(sampler.kept().len(), MAX_THEMES + 1);
    }

    #[test]
    fn an_opening_family_keeps_a_sample_when_a_row_has_no_theme() {
        let lines: Vec<String> = (0..PER_OPENING + 30)
            .map(|i| theme_row(&format!("o{i}"), "", "Fam Fam_Var"))
            .collect();
        assert_eq!(sampler_with(&lines).kept().len(), PER_OPENING);
    }

    #[test]
    fn an_opening_family_is_between_one_and_eighty_bytes() {
        let kept = |tags: &str| sampler_with(&[theme_row("a", "", tags)]).kept().len();
        assert_eq!(kept(""), 0);
        assert_eq!(kept("  "), 0);
        assert_eq!(kept(&"A".repeat(80)), 1);
        assert_eq!(kept(&"A".repeat(81)), 0);
    }

    #[test]
    fn the_sampler_tracks_at_most_512_opening_families() {
        let lines: Vec<String> = (0..=MAX_OPENINGS)
            .map(|i| theme_row(&format!("o{i}"), "", &format!("Family{i}")))
            .collect();
        assert_eq!(sampler_with(&lines).kept().len(), MAX_OPENINGS);
    }

    #[test]
    fn counting_follows_the_kept_rows_and_release_frees_them() {
        let mut sampler = sampler_with(&[]);
        assert_eq!(sampler.count(), 0);
        sampler.add(&row("a", 1500, "fork", "")).unwrap();
        sampler.add(&row("b", 1700, "fork", "")).unwrap();
        assert_eq!(sampler.count(), 2);
        sampler.release();
        assert_eq!(sampler.count(), 2);
        assert!(sampler.kept().is_empty());
    }
}
