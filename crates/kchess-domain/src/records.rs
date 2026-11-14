//! Game review keys, ratings, studies, game results, time controls, accounts, Lichess events and
//! errors, OAuth looks and analysis contexts.

pub mod accounts;
pub mod challenge;
pub mod context;
pub mod games;
pub mod lichess;
pub mod oauth;
pub mod online;
pub mod ratings;
pub mod review;
pub mod studies;
pub mod time_control;

/// The label of each rating category.
pub use ratings::DISPLAY_NAMES as RATING_DISPLAY_NAMES;

#[cfg(test)]
mod tests {
    use super::{ratings, review};
    use crate::rules::INITIAL_FEN;
    use crate::variant::Variant;

    /// A review is kept by its moves and start; a variant not played by standard rules makes the
    /// same moves another game, and Chess960 (standard rules) does not.
    #[test]
    fn a_review_key_names_a_variant_only_when_its_rules_differ() {
        let key = |variant| review::review_key(INITIAL_FEN, &["e2e4", "e7e5"], variant);
        let standard = key(None);
        assert_eq!(key(Some(Variant::Standard)), standard);
        assert_eq!(key(Some(Variant::Chess960)), standard);
        let atomic = key(Some(Variant::Atomic));
        assert_ne!(atomic, standard);
        assert_ne!(key(Some(Variant::Crazyhouse)), atomic);
        assert_ne!(review::review_key(INITIAL_FEN, &["e2e4"], None), standard);
    }

    /// A rating category's label by its key; none for a key that is not one.
    #[test]
    fn a_rating_category_is_labelled_by_its_key() {
        for (key, label) in ratings::DISPLAY_NAMES {
            assert_eq!(ratings::perf_label(key), Some(*label));
        }
        assert_eq!(ratings::perf_label("blitz"), Some("Blitz"));
        assert_eq!(ratings::perf_label("puzzle"), Some("Puzzles"));
        assert_eq!(ratings::perf_label("bughouse"), None);
    }
}
