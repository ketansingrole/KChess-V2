//! Game sessions as every KChess frontend drives them: each decision is the domain's typed
//! session (`kchess_domain::games`); these drivers answer what it asks with the frontend's values
//! and hand its effects to the frontend, in order. The GPUI app and the CLI share them.

pub mod archive;
pub mod computer;
pub mod history;
pub mod local;
pub mod puzzle;

use serde_json::Value;
pub use shakmaty::Color;

/// A random number in [0, 1), like `Math.random()`, for the rules that draw from them.
pub fn random_unit() -> f64 {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64
}

/// A Chess960 start drawn at random: its number, 0 to 959.
pub fn random_chess960() -> i64 {
    ((random_unit() * 960.) as i64).min(959)
}

/// The side asked for: `white`, `black`, or either at random for `random`.
pub fn draw_color(choice: &str) -> Color {
    match choice {
        "random" if random_unit() < 0.5 => Color::White,
        "random" | "black" => Color::Black,
        _ => Color::White,
    }
}

/// A side as the sessions name it.
pub fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "white",
        Color::Black => "black",
    }
}

/// Milliseconds since the epoch (`Date.now()`).
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_secs_f64() * 1000.)
}

/// How a game ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Wins(Color),
    Draw,
}

impl Outcome {
    /// The result as a PGN `Result` tag reads it; `*` while the game goes on.
    pub fn tag(outcome: Option<Outcome>) -> &'static str {
        match outcome {
            None => "*",
            Some(Outcome::Wins(Color::White)) => "1-0",
            Some(Outcome::Wins(Color::Black)) => "0-1",
            Some(Outcome::Draw) => "1/2-1/2",
        }
    }

    /// A PGN `Result` tag's outcome; `None` for `*` or anything else.
    pub fn from_tag(tag: &str) -> Option<Outcome> {
        match tag {
            "1-0" => Some(Outcome::Wins(Color::White)),
            "0-1" => Some(Outcome::Wins(Color::Black)),
            "1/2-1/2" => Some(Outcome::Draw),
            _ => None,
        }
    }
}

/// A move typed as SAN ("Nf3", "O-O", "exd8=Q") or UCI ("g1f3") in the position `fen` of
/// `variant`, as UCI when it is legal there.
pub fn typed_uci(variant: &str, fen: &str, text: &str) -> Option<String> {
    let text = text.trim();
    let position = kchess_domain::rules::setup_start(variant, fen)?;
    let bytes = text.as_bytes();
    let is_uci = bytes.len() >= 4
        && bytes[0].is_ascii_lowercase()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_lowercase()
        && bytes[3].is_ascii_digit();
    if is_uci {
        kchess_domain::position::play_uci(&position, text).map(|_| text.to_lowercase())
    } else {
        kchess_domain::position::play_san(&position, text).map(|played| played.uci)
    }
}

/// A count or index from the rules, which write JavaScript numbers: `2` and `2.0` alike.
pub fn whole(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.).map(|n| n as u64))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A typed move is read as UCI when it looks like one, and as SAN otherwise, in the
    /// position given; anything illegal there is none.
    #[test]
    fn a_typed_move_is_read_as_san_or_uci() {
        let start = kchess_domain::rules::INITIAL_FEN;
        assert_eq!(typed_uci("standard", start, "Nf3").as_deref(), Some("g1f3"));
        assert_eq!(
            typed_uci("standard", start, " g1f3 ").as_deref(),
            Some("g1f3")
        );
        assert_eq!(typed_uci("standard", start, "e2e5"), None);
        assert_eq!(typed_uci("standard", start, "Nf6"), None);
        let castle = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert_eq!(
            typed_uci("standard", castle, "O-O").as_deref(),
            Some("e1h1")
        );
        let zh = "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        assert_eq!(typed_uci("crazyhouse", zh, "P@e3").as_deref(), Some("P@e3"));
    }

    #[test]
    fn outcomes_read_and_write_as_pgn_tags() {
        for outcome in [
            None,
            Some(Outcome::Wins(Color::White)),
            Some(Outcome::Wins(Color::Black)),
            Some(Outcome::Draw),
        ] {
            assert_eq!(Outcome::from_tag(Outcome::tag(outcome)), outcome);
        }
        assert_eq!(Outcome::tag(Some(Outcome::Draw)), "1/2-1/2");
    }

    #[test]
    fn colours_and_chess960_starts_are_drawn_as_asked() {
        assert_eq!(draw_color("white"), Color::White);
        assert_eq!(draw_color("black"), Color::Black);
        let draws: Vec<Color> = (0..64).map(|_| draw_color("random")).collect();
        assert!(draws.contains(&Color::White) && draws.contains(&Color::Black));
        assert!((0..256).all(|_| (0..960).contains(&random_chess960())));
    }

    #[test]
    fn random_units_fall_in_zero_to_one_and_vary() {
        let draws: Vec<f64> = (0..64).map(|_| random_unit()).collect();
        assert!(draws.iter().all(|n| (0.0..1.0).contains(n)), "{draws:?}");
        assert!(draws.iter().any(|n| *n > 0.5) && draws.iter().any(|n| *n < 0.5));
    }

    #[test]
    fn whole_reads_integers_and_floats_alike() {
        assert_eq!(whole(&json!(2)), 2);
        assert_eq!(whole(&json!(2.0)), 2);
        assert_eq!(whole(&json!(0.0)), 0);
        assert_eq!(whole(&json!(-3.0)), 0);
        assert_eq!(whole(&json!(-1)), 0);
        assert_eq!(whole(&json!("2")), 0);
        assert_eq!(whole(&Value::Null), 0);
    }

    #[test]
    fn random_unit_stays_in_range() {
        for _ in 0..1_000 {
            let n = random_unit();
            assert!((0. ..1.).contains(&n), "{n}");
        }
    }
}
