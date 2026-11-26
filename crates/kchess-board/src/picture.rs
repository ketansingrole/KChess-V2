//! A picture of a chess board to its position (`kchess_recognise`), compared with every bundled
//! piece set. Slow enough to run off the main thread: callers run it with `background_spawn`.

use std::sync::LazyLock;

use kchess_recognise::{PieceSet, Recogniser};
pub use kchess_recognise::{Recognised, Refusal};
use shakmaty::{Color, Piece, Role};

use crate::board::looks;

/// Every bundled piece set, drawn as references the first time a picture is read at each size.
static RECOGNISER: LazyLock<Recogniser> = LazyLock::new(|| {
    let sets = looks::piece_sets()
        .iter()
        .map(|set| PieceSet {
            name: set.id.clone(),
            pieces: Color::ALL
                .into_iter()
                .flat_map(|color| Role::ALL.into_iter().map(move |role| Piece { color, role }))
                .filter_map(|piece| {
                    crate::assets::shared(&looks::piece_path(&set.id, piece))
                        .map(|svg| (piece, svg.into_owned()))
                })
                .collect(),
        })
        .collect();
    Recogniser::new(sets)
});

/// The position in a picture (PNG, JPEG, GIF, WebP, BMP or TIFF) of a 2D board.
pub fn read_board_picture(bytes: &[u8]) -> Result<Recognised, Refusal> {
    RECOGNISER.read(bytes)
}

/// Whether a file's name is a picture's, by its extension.
pub fn is_picture_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

#[cfg(test)]
mod corpus;

#[cfg(test)]
mod tests {
    use super::corpus::{CHANGES, Case, Change, run_all, table};

    /// Every set on every board at every size with every change, `per` positions each.
    fn cases(sizes: &[u32], changes: &[Change], per: u64) -> Vec<Case> {
        let mut cases = Vec::new();
        let mut seed = 0;
        for set in crate::board::looks::piece_sets() {
            for board in crate::board::looks::boards() {
                for &size in sizes {
                    for &change in changes {
                        for _ in 0..per {
                            seed += 1;
                            cases.push(Case {
                                set: set.id.clone(),
                                board: board.id.clone(),
                                size,
                                change,
                                seed,
                            });
                        }
                    }
                }
            }
        }
        cases
    }

    /// What every reading must reach: the board found in every picture, 99% of each set's
    /// squares and 95% of all positions read right.
    fn assert_reads_well(outcomes: &[super::corpus::Outcome]) {
        assert!(
            outcomes.iter().all(|outcome| outcome.found),
            "a board was not found"
        );
        for set in crate::board::looks::piece_sets() {
            let of_set: Vec<_> = outcomes.iter().filter(|o| o.case.set == set.id).collect();
            let right: usize = of_set.iter().map(|o| o.squares_right).sum();
            let share = right as f64 / (64 * of_set.len()).max(1) as f64;
            assert!(share >= 0.99, "{}: {:.2}% of squares", set.id, share * 100.);
        }
        let right = outcomes.iter().filter(|o| o.position_right).count();
        let share = right as f64 / outcomes.len() as f64;
        assert!(share >= 0.95, "{:.1}% of positions", share * 100.);
    }

    /// Every piece set on every board theme, once each, the sizes and changes taken in turn: 143
    /// pictures. Measured 2026-11-26: every board found, 99.93% of squares and 98.6% of positions
    /// read right (the least, pixel: 99.52% of squares); the sweep below has the numbers by set,
    /// board, size and change.
    #[test]
    fn every_set_on_every_board_is_read() {
        let changes = CHANGES;
        let sizes = [240, 400, 640];
        let mut seed = 10_000;
        let mut cases = Vec::new();
        for set in crate::board::looks::piece_sets() {
            for board in crate::board::looks::boards() {
                seed += 1;
                cases.push(Case {
                    set: set.id.clone(),
                    board: board.id.clone(),
                    size: sizes[seed as usize % sizes.len()],
                    change: changes[seed as usize % changes.len()],
                    seed,
                });
            }
        }
        let outcomes = run_all(cases);
        eprintln!("{}", table(&outcomes, "set", |case| case.set.clone()));
        assert_reads_well(&outcomes);
    }

    /// The export's own picture of the start position, as bytes, from Black's side.
    #[test]
    fn an_exported_picture_is_read() {
        let options = crate::game_image::ImageOptions {
            setup: kchess_domain::variant::Setup::standard(),
            moves: Vec::new(),
            orientation: shakmaty::Color::Black,
            board_theme: "brown".into(),
            piece_set: "cburnett".into(),
            white: String::new(),
            black: String::new(),
            size: 320,
        };
        let png = crate::game_image::position_png(&options, 0).unwrap();
        let read = super::read_board_picture(&png).unwrap();
        assert_eq!(
            read.fen,
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        assert!(!read.white_at_bottom);
        assert_eq!(
            super::read_board_picture(b"no"),
            Err(super::Refusal::Unreadable)
        );
        assert!(super::is_picture_file(std::path::Path::new("board.PNG")));
        assert!(super::is_picture_file(std::path::Path::new("a/b.jpeg")));
        assert!(!super::is_picture_file(std::path::Path::new("game.pgn")));
    }

    /// The whole sweep, 2145 pictures (every set on every board at 240, 400 and 640 pixels, with
    /// each change): `cargo test -p kchess-board picture::tests::sweep -- --ignored --nocapture`
    /// (half a minute; the recogniser is optimized in test builds too).
    ///
    /// Measured 2026-11-26, squares and positions read right (every board found):
    ///
    /// | set       | squares | positions |   | change      | squares | positions |
    /// | --------- | ------- | --------- | - | ----------- | ------- | --------- |
    /// | cburnett  | 100.00% | 100.0%    |   | as drawn    | 100.00% | 99.8%     |
    /// | merida    | 99.99%  | 99.5%     |   | coordinates | 99.96%  | 98.4%     |
    /// | chessnut  | 100.00% | 100.0%    |   | margin      | 99.99%  | 99.5%     |
    /// | pirouetti | 99.85%  | 93.3%     |   | scaled      | 99.75%  | 95.3%     |
    /// | letter    | 99.93%  | 95.9%     |   | JPEG        | 99.96%  | 97.7%     |
    /// | mpchess   | 99.98%  | 98.5%     |   |             |         |           |
    /// | fantasy   | 99.99%  | 99.5%     |   | 240 px      | 99.82%  | 95.8%     |
    /// | spatial   | 99.97%  | 98.5%     |   | 400 px      | 99.97%  | 98.9%     |
    /// | celtic    | 99.99%  | 99.5%     |   | 640 px      | 100.00% | 99.7%     |
    /// | rhosgfx   | 99.98%  | 99.0%     |   |             |         |           |
    /// | pixel     | 99.57%  | 95.9%     |   | all         | 99.93%  | 98.1%     |
    ///
    /// 83 of the 95 squares read wrongly were marked doubtful. The misses gather at the smallest
    /// squares: pixel art and Pirouetti's outline-less white pieces on light squares, scaled to
    /// 25-pixel squares.
    #[test]
    #[ignore = "a long measurement"]
    fn sweep() {
        let started = std::time::Instant::now();
        // `KCHESS_RECOGNISE_SET=<set>` measures one piece set alone.
        // `KCHESS_RECOGNISE_CASE=<seed>` reads one picture alone, as the dumps name them.
        let only = kchess_host::env::var("KCHESS_RECOGNISE_SET");
        let case = kchess_host::env::var("KCHESS_RECOGNISE_CASE");
        let outcomes = run_all(
            cases(&[240, 400, 640], &CHANGES, 1)
                .into_iter()
                .filter(|case| only.as_ref().is_none_or(|set| *set == case.set))
                .filter(|it| {
                    case.as_ref()
                        .is_none_or(|seed| *seed == it.seed.to_string())
                })
                .collect(),
        );
        eprintln!("{} pictures in {:?}", outcomes.len(), started.elapsed());
        eprintln!("{}", table(&outcomes, "set", |case| case.set.clone()));
        eprintln!("{}", table(&outcomes, "board", |case| case.board.clone()));
        eprintln!("{}", table(&outcomes, "size", |case| case.size.to_string()));
        eprintln!(
            "{}",
            table(&outcomes, "change", |case| format!("{:?}", case.change))
        );
        eprintln!("{}", table(&outcomes, "all", |_| "all".into()));
        if only.is_none() && case.is_none() {
            assert_reads_well(&outcomes);
        }
    }

    /// Reads the pictures `KCHESS_RECOGNISE_FILES` names (separated by `:`), printing each one's
    /// position or refusal: for trying the reader on pictures that cannot be in the repository.
    #[test]
    #[ignore = "reads files named in KCHESS_RECOGNISE_FILES"]
    fn read_files() {
        let files = std::env::var("KCHESS_RECOGNISE_FILES").unwrap_or_default();
        for path in files.split(':').filter(|path| !path.is_empty()) {
            let bytes = std::fs::read(path).unwrap();
            match super::read_board_picture(&bytes) {
                Ok(read) => println!("{path}\t{}\tdoubtful {:?}", read.fen, read.doubtful),
                Err(refusal) => println!("{path}\trefused: {refusal}"),
            }
        }
    }
}
