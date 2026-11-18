//! What every KChess frontend does around the domain's typed sessions (`kchess_domain::games`):
//! the randomness and the clock the rules leave to the host, the Game history writes, a puzzle on
//! the board and broadcast rounds. The GPUI app and the CLI share them.

pub mod broadcast;
pub mod history;
pub mod puzzle;

use shakmaty::Color;

/// A random number in [0, 1); an error when the system has no randomness to give.
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

/// Milliseconds since the epoch.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_secs_f64() * 1000.)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Chess960 start is any of the 960, drawn afresh each time.
    #[test]
    fn a_chess960_start_is_drawn_from_all_960() {
        let draws: Vec<i64> = (0..64).map(|_| random_chess960().unwrap()).collect();
        assert!(
            draws.iter().all(|draw| (0..960).contains(draw)),
            "{draws:?}"
        );
        let mut distinct = draws.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(distinct.len() > 8, "{draws:?}");
        assert!(draws.iter().any(|&draw| draw > 1), "{draws:?}");
    }

    /// The clock reads milliseconds since the epoch, as the system clock does.
    #[test]
    fn now_is_milliseconds_since_the_epoch() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let now = now_ms();
        assert!(now >= before && now < before + 60_000., "{now} vs {before}");
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
