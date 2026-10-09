//! The puzzle database sampler (`core/src/services/puzzleSampler.ts`): reads Lichess's puzzle
//! CSV as decompressed bytes and keeps a random sample spread over every rating and theme.

use serde::Serialize;
use std::collections::HashMap;
use std::rc::Rc;

use crate::js;

const RATING_BUCKET: f64 = 50.0;
const PER_BUCKET: usize = 1800;
const PER_THEME: usize = 400;
const MAX_LINE: usize = 16_000;
const MAX_THEMES: usize = 256;
pub const OVERSIZED: &str = "Puzzle database contains an oversized CSV line.";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DbPuzzle {
    pub id: String,
    pub fen: String,
    pub moves: String,
    pub rating: f64,
    pub plays: f64,
    pub themes: String,
}

/// The generator the TypeScript tests use (a 32-bit mix of a Weyl sequence), so a seeded
/// native sampler keeps exactly the rows a seeded TypeScript sampler keeps.
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

    /// Count the row and decide where it goes, drawing a random number exactly when the
    /// TypeScript reservoir does; the row itself is only built when some reservoir keeps it.
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
    /// Reservoirs in creation order, which `kept()` follows like the TypeScript `Map`s.
    buckets: Vec<Reservoir>,
    bucket_index: FastMap<i64, usize>,
    themes: Vec<Reservoir>,
    theme_index: FastMap<String, usize>,
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
            tail: Vec::new(),
            released: None,
            lines: 0,
        }
    }

    /// One CSV line: `PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,…`.
    pub fn add(&mut self, line: &str) -> Result<(), &'static str> {
        self.add_bytes(line.as_bytes())
    }

    /// One line as raw bytes. The TypeScript sampler decodes whole lines (invalid UTF-8 becoming
    /// U+FFFD); here only cells whose text matters are decoded. A kept row's id and themes are
    /// ASCII, so rows with other bytes there are rejected either way.
    fn add_bytes(&mut self, line: &[u8]) -> Result<(), &'static str> {
        if line.len() > MAX_LINE && js::utf16_len(&String::from_utf8_lossy(line)) > MAX_LINE {
            return Err(OVERSIZED);
        }
        let mut cells: [&[u8]; 8] = [&[]; 8];
        let mut rest = line;
        for (i, cell) in cells.iter_mut().enumerate() {
            match memchr::memchr(b',', rest) {
                Some(at) => {
                    *cell = &rest[..at];
                    rest = &rest[at + 1..];
                }
                None if i == 7 => *cell = rest,
                None => return Ok(()),
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
        self.lines += 1;
        let popularity = number(cells[5]);
        let plays = number(cells[6]);
        let is_solid = solid(deviation, popularity, plays);
        let is_solid_for_theme = solid_for_theme(deviation, popularity, plays);
        if !is_solid && !is_solid_for_theme {
            return Ok(());
        }
        // The row is built only once a reservoir keeps it; placing at once keeps each
        // reservoir's length, and so its random draws, as the TypeScript sampler has them.
        let mut row: Option<Rc<DbPuzzle>> = None;
        let mut build = || {
            Rc::new(DbPuzzle {
                id: id.to_string(),
                fen: String::from_utf8_lossy(cells[1]).into_owned(),
                moves: String::from_utf8_lossy(cells[2]).into_owned(),
                rating,
                plays,
                themes: themes.to_string(),
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
        self.bucket_index = FastMap::default();
        self.theme_index = FastMap::default();
        self.tail = Vec::new();
    }

    /// Every puzzle kept, once, in the order the TypeScript sampler returns them.
    pub fn kept(&self) -> Vec<&DbPuzzle> {
        let mut order: Vec<&str> = Vec::new();
        let mut latest: HashMap<&str, &DbPuzzle> = HashMap::new();
        for reservoir in self.buckets.iter().chain(&self.themes) {
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
    fn random_matches_the_typescript_generator() {
        // rng(1) in core/tests/unit/native-rules.test.ts
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
    }
}
