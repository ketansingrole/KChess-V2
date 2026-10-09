//! KChess rules shared by every frontend through the core: chess rules with chessops
//! semantics, PGN, the analysis tree, game replay and library document decoding.

pub mod api;
pub mod games;
pub mod js;
pub mod library;
pub mod misc;
pub mod online_game;
pub mod pgn;
pub mod position;
pub mod puzzle;
pub mod records;
pub mod replay;
pub mod review;
pub mod rules;
pub mod trainer;
pub mod training;
pub mod tree;
pub mod voice;
