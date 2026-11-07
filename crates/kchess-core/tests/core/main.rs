//! The core's integration tests as one test binary: each file here was its own binary, and linking
//! some forty of them made every change to the core wait for the linker (2026-11-06: 45 s to
//! rebuild them after an edit). Tests that set process-wide switches (`kchess_core::env::set`)
//! stay binaries of their own beside this directory, since the tests here share one process under
//! `cargo test`.

#[path = "../support/lichess_http.rs"]
mod fixtures;
#[path = "../support/signed_in.rs"]
mod signed_in;
#[path = "../support/mod.rs"]
mod support;

mod chesscom_sync;
mod core_startup;
mod domain_dispatch;
mod engine_admission;
mod engine_analysis;
mod engine_options;
mod engine_review;
mod engine_review_stockfish;
mod engine_scheduler;
mod engine_search;
mod engine_services_stockfish;
mod engine_status_variant;
mod engine_stockfish;
mod engine_variant_fairy;
mod facade_dispatch;
mod facade_lichess;
mod facade_sync_play_time;
mod facade_tournaments;
mod lichess_accounts;
mod lichess_client;
mod lichess_endpoints;
mod lichess_lookups;
mod lichess_oauth;
mod lichess_online;
mod lichess_policy;
mod lichess_puzzles;
mod lichess_reviews;
mod lichess_studies;
mod lichess_tournaments;
mod lichess_watch;
mod profile_lock;
mod review_admission;
mod store_accounts;
mod store_games;
mod store_import;
mod store_insights;
mod store_library_scale;
mod store_reviews;
mod uci_lifecycle;
mod voice_model;
mod voice_ownership;
mod voice_recognizer;
