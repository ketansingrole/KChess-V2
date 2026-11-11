//! Game sessions as every KChess frontend drives them: the domain's typed sessions
//! (`kchess_domain::games`), which the frontend calls with what it knows and whose effects it
//! carries out, in order, and the frontend's part of them: randomness, the clock and the Game
//! history writes. The GPUI app and the CLI share them.

pub mod computer;
pub mod history;
pub mod local;
pub mod puzzle;

pub use kchess_domain::games::Outcome;
pub use shakmaty::Color;

use kchess_domain::variant::Variant;

/// A random number in [0, 1), like `Math.random()`; an error when the system has no randomness
/// to give (it used to draw 0 every time).
pub fn random_unit() -> Result<f64, String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| format!("No random numbers: {error}"))?;
    Ok((u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64)
}

/// A Chess960 start drawn at random: its number, 0 to 959.
pub fn random_chess960() -> Result<i64, String> {
    Ok(((random_unit()? * 960.) as i64).min(959))
}

/// The side asked for: `white`, `black`, or either at random for `random`.
pub fn draw_color(choice: &str) -> Result<Color, String> {
    Ok(match choice {
        "random" if random_unit()? < 0.5 => Color::White,
        "random" | "black" => Color::Black,
        _ => Color::White,
    })
}

/// Milliseconds since the epoch (`Date.now()`).
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_secs_f64() * 1000.)
}

/// A move typed as SAN ("Nf3", "O-O", "exd8=Q") or UCI ("g1f3") in the position `fen` of
/// `variant`, as UCI when it is legal there.
pub fn typed_uci(variant: Variant, fen: &str, text: &str) -> Option<String> {
    let text = text.trim();
    let position = kchess_domain::rules::setup_start(variant, fen)?;
    let uci = text.to_lowercase();
    if kchess_domain::rules::is_uci_text(&uci) {
        kchess_domain::position::play_uci(&position, &uci).map(|_| uci)
    } else {
        kchess_domain::position::play_san(&position, text).map(|played| played.uci)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typed move is read as UCI when it looks like one, and as SAN otherwise, in the
    /// position given; anything illegal there is none.
    #[test]
    fn a_typed_move_is_read_as_san_or_uci() {
        let start = kchess_domain::rules::INITIAL_FEN;
        assert_eq!(
            typed_uci(Variant::Standard, start, "Nf3").as_deref(),
            Some("g1f3")
        );
        assert_eq!(
            typed_uci(Variant::Standard, start, " g1f3 ").as_deref(),
            Some("g1f3")
        );
        assert_eq!(typed_uci(Variant::Standard, start, "e2e5"), None);
        assert_eq!(typed_uci(Variant::Standard, start, "Nf6"), None);
        let castle = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert_eq!(
            typed_uci(Variant::Standard, castle, "O-O").as_deref(),
            Some("e1h1")
        );
        let zh = "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        assert_eq!(
            typed_uci(Variant::Crazyhouse, zh, "P@e3").as_deref(),
            Some("P@e3")
        );
    }

    #[test]
    fn colours_and_chess960_starts_are_drawn_as_asked() {
        assert_eq!(draw_color("white"), Ok(Color::White));
        assert_eq!(draw_color("black"), Ok(Color::Black));
        let draws: Vec<Color> = (0..64).map(|_| draw_color("random").unwrap()).collect();
        assert!(draws.contains(&Color::White) && draws.contains(&Color::Black));
        assert!((0..256).all(|_| (0..960).contains(&random_chess960().unwrap())));
    }

    #[test]
    fn random_units_fall_in_zero_to_one_and_vary() {
        let draws: Vec<f64> = (0..1_000).map(|_| random_unit().unwrap()).collect();
        assert!(draws.iter().all(|n| (0.0..1.0).contains(n)), "{draws:?}");
        assert!(draws.iter().any(|n| *n > 0.5) && draws.iter().any(|n| *n < 0.5));
    }
}
