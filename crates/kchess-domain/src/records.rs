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
