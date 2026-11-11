//! The core's integration tests as one test binary: each file here was its own binary, and linking
//! some forty of them made every change to the core wait for the linker (2026-11-06: 45 s to
//! rebuild them after an edit). Tests that need the whole process (a panic hook, a counting
//! allocator) stay binaries of their own beside this directory.

#[path = "../support/lichess_http.rs"]
mod fixtures;
#[path = "../support/signed_in.rs"]
mod signed_in;
#[path = "../support/mod.rs"]
mod support;

mod ai_connection;
mod ai_effort;
mod chesscom_sync;
mod core_close;
mod core_close_lichess;
mod core_migration;
mod core_startup;
mod core_wiring;
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
mod explorer_assistance;
mod explorer_shutdown;
mod facade_accounts;
mod facade_challenges;
mod facade_dispatch;
mod facade_effects;
mod facade_engine;
mod facade_lichess;
mod facade_online;
mod facade_online_play;
mod facade_puzzles;
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
mod lichess_store_fake;
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
mod support_waits;
mod uci_lifecycle;
mod voice_model;
mod voice_model_facade;
mod voice_ownership;
mod voice_recognizer;
mod wire_accounts;
mod wire_library;
mod wire_play;
