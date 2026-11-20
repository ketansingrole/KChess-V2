//! KChess rules shared by every frontend through the core: chess rules,
//! PGN, the analysis tree, game replay and library document decoding.

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod calendar;
pub mod chesscom;
pub mod coach;
pub mod color;
pub mod editor;
pub mod embedded;
pub mod games;
pub mod js;
pub mod keyed;
pub mod library;
pub mod links;
pub mod material;
pub mod misc;
pub mod online_game;
pub mod perf;
pub mod pgn;
pub mod position;
pub mod puzzle;
pub mod records;
pub mod replay;
pub mod review;
pub mod rules;
pub mod squares;
pub mod tactics;
pub mod theme;
pub mod time_use;
pub mod trainer;
pub mod training;
pub mod tree;
pub mod variant;
pub mod voice;
mod wire;
