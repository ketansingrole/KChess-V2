//! Lichess, migrating from `core/src/services/{lichess,requestPolicy,ndjson,oauthPage,usage,
//! challenges,tournaments,spectate,studies,cloudEval,positionLookup}.ts` (see RUST_MIGRATION.md).

pub mod accounts;
pub mod challenges;
pub mod client;
pub mod lookups;
pub mod oauth;
pub mod online;
pub mod policy;
pub mod puzzles;
pub mod reviews;
pub mod stream;
pub mod studies;
pub mod tournaments;
pub mod watch;
