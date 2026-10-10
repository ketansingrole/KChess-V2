//! `@kchess/native`: the Rust rules for Node and Electron hosts. Values cross as JSON text
//! (see `crates/kchess-node/js/native.ts`); `null` stands for the TypeScript `undefined`.

mod core;
pub mod voice;

use kchess_domain::{api, library, pgn, puzzle, replay, review, tree};
use napi_derive::napi;
use serde_json::Value;

fn to_json<T: serde::Serialize>(value: &T) -> napi::Result<String> {
    serde_json::to_string(value).map_err(|e| napi::Error::from_reason(e.to_string()))
}

/// Parse a stored document; text that is not JSON decodes to nothing, as in the core.
fn parse(json: &str) -> Option<Value> {
    serde_json::from_str(json).ok()
}

/// `api::call`: any rules method by name, with a JSON array of arguments and a JSON result.
#[napi(catch_unwind)]
pub fn invoke(method: String, args: String) -> napi::Result<String> {
    api::call(&method, &args).map_err(napi::Error::from_reason)
}

#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[napi(catch_unwind)]
pub fn replay_setup(
    variant: String,
    fen: String,
    moves: Vec<String>,
) -> napi::Result<Option<String>> {
    replay::replay_setup(&variant, &fen, &moves)
        .map(|r| to_json(&r))
        .transpose()
}

#[napi(catch_unwind)]
pub fn replay_positions(fen: String, moves: Vec<String>) -> napi::Result<String> {
    to_json(&replay::replay_positions(&fen, &moves))
}

#[napi(catch_unwind)]
pub fn tree_from_pgn(pgn: String) -> napi::Result<Option<String>> {
    tree::tree_from_pgn(&pgn).map(|t| to_json(&t)).transpose()
}

#[napi(catch_unwind)]
pub fn valid_pgn(pgn: String) -> bool {
    tree::valid_pgn(&pgn)
}

#[napi(catch_unwind)]
pub fn decode_archived_game(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| library::decode_archived_game(&raw))
        .map(|g| to_json(&g))
        .transpose()
}

/// `decodeStudy`: one study (as restored after removal), or null when any chapter is invalid.
#[napi(catch_unwind)]
pub fn decode_study(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| library::decode_study(&raw))
        .map(|s| to_json(&s))
        .transpose()
}

/// `decodeArchive`: the one-document game archive of earlier releases.
#[napi(catch_unwind)]
pub fn decode_archive(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| library::decode_archive(&raw))
        .map(|g| to_json(&g))
        .transpose()
}

#[napi(catch_unwind)]
pub fn decode_studies(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| library::decode_studies(&raw))
        .map(|s| to_json(&s))
        .transpose()
}

#[napi(catch_unwind)]
pub fn decode_mistakes(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| library::decode_mistakes(&raw))
        .map(|m| to_json(&m))
        .transpose()
}

/* ── Phase 2: reviews, study PGN and the puzzle sampler ── */

/// The `Math.exp` the review rules use; exposed so tests can compare it with V8's on each platform.
#[napi]
pub fn js_exp(x: f64) -> f64 {
    review::js_exp(x)
}

/// `analyseReview`; null when the stored review needs the TypeScript rules.
#[napi(catch_unwind)]
pub fn analyse_review(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| review::analyse_review(&raw))
        .map(|a| to_json(&a))
        .transpose()
}

/// `summarize`; null when the stored review needs the TypeScript rules.
#[napi(catch_unwind)]
pub fn summarize_review(json: String) -> napi::Result<Option<String>> {
    parse(&json)
        .and_then(|raw| review::summarize(&raw))
        .map(|s| to_json(&s))
        .transpose()
}

/// `lichessLine`: `{ fen, moves }`.
#[napi(catch_unwind)]
pub fn lichess_line(
    moves: String,
    pgn: Option<String>,
    initial_fen: Option<String>,
) -> napi::Result<String> {
    let (fen, moves) = review::lichess_line(&moves, pgn.as_deref(), initial_fen.as_deref());
    to_json(&serde_json::json!({ "fen": fen, "moves": moves }))
}

#[napi(catch_unwind)]
pub fn san_to_uci(fen: String, sans: Vec<String>) -> Vec<String> {
    review::san_to_uci(&fen, &sans)
}

/// `studyDocumentPgn` over `[name, pgn]` chapters; null when a chapter holds no game.
#[napi(catch_unwind)]
pub fn study_document_pgn(chapters: Vec<Vec<String>>) -> Option<String> {
    let pairs: Option<Vec<(&str, &str)>> = chapters
        .iter()
        .map(|c| match c.as_slice() {
            [name, pgn] => Some((name.as_str(), pgn.as_str())),
            _ => None,
        })
        .collect();
    pgn::study_document_pgn(&pairs?)
}

/// Whether a study still matches its downloaded Lichess copy; null when a chapter holds no game.
#[napi(catch_unwind)]
pub fn study_matches_cloud(chapters: Vec<Vec<String>>, downloaded: String) -> Option<bool> {
    let pairs: Option<Vec<(&str, &str)>> = chapters
        .iter()
        .map(|c| match c.as_slice() {
            [name, pgn] => Some((name.as_str(), pgn.as_str())),
            _ => None,
        })
        .collect();
    pgn::study_matches(&pairs?, &downloaded)
}

#[napi(catch_unwind)]
pub fn study_content(pgn: String) -> String {
    pgn::study_content(&pgn)
}

/// The puzzle database sampler: push decompressed CSV bytes, then `finish` and take `kept`.
#[napi]
pub struct PuzzleSampler {
    inner: puzzle::Sampler,
}

#[napi]
impl PuzzleSampler {
    #[napi(constructor)]
    pub fn new(seed: u32) -> Self {
        PuzzleSampler {
            inner: puzzle::Sampler::new(puzzle::Random::new(seed)),
        }
    }

    #[napi]
    pub fn push(&mut self, chunk: &[u8]) -> napi::Result<()> {
        self.inner.push(chunk).map_err(napi::Error::from_reason)
    }

    #[napi]
    pub fn finish(&mut self) -> napi::Result<()> {
        self.inner.finish().map_err(napi::Error::from_reason)
    }

    #[napi(getter)]
    pub fn lines(&self) -> f64 {
        self.inner.lines as f64
    }

    #[napi(getter)]
    pub fn count(&self) -> u32 {
        self.inner.count() as u32
    }

    /// JSON array of the kept `DbPuzzle` rows.
    #[napi]
    pub fn kept(&self) -> napi::Result<String> {
        to_json(&self.inner.kept())
    }

    /// Free the sample (the counts stay); call after `kept`.
    #[napi]
    pub fn release(&mut self) {
        self.inner.release();
    }
}
