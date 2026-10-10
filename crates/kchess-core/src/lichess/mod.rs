//! Lichess, migrating from `crates/kchess-node/js/{lichess,requestPolicy,ndjson,oauthPage,usage,
//! challenges,tournaments,spectate,studies,cloudEval,positionLookup}.ts` (see RUST_MIGRATION.md).

pub mod accounts;
pub mod adapters;
pub mod challenges;
pub mod client;
pub mod lookups;
pub mod oauth;
pub mod online;
pub mod policy;
pub mod puzzles;
pub mod reviews;
pub mod services;
pub mod stream;
pub mod studies;
pub mod tournaments;
pub mod watch;
