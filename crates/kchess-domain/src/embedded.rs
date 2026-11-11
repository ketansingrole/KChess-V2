//! The data built into KChess (themes, openings, lessons, the training tables), read once.

use serde::de::DeserializeOwned;

/// The built-in `what`, read from its JSON `text`. The data is part of the program and a test
/// reads every file, so a file that does not read is a broken build, not a state to show: it
/// panics rather than leave a page silently empty.
pub fn parse<T: DeserializeOwned>(what: &str, text: &str) -> T {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("the built-in {what}: {error}"))
}

#[cfg(test)]
mod tests {
    /// Every built-in data file reads.
    #[test]
    fn the_built_in_data_reads() {
        assert!(
            crate::theme::presets()
                .as_array()
                .is_some_and(|themes| !themes.is_empty())
        );
        assert!(!crate::training::lessons::data().courses.is_empty());
        assert!(!crate::training::data::data().engine_ladder.is_empty());
    }

    #[test]
    #[should_panic(expected = "the built-in lessons")]
    fn broken_data_is_a_broken_build() {
        super::parse::<Vec<u32>>("lessons", "[1,");
    }
}
