//! The board and the games on it, over GPUI: the board (`board`), opening a game on the analysis
//! board from any page (`analysis`), game sessions as views drive them (`game`), notation as
//! Settings shows it (`notation`), move and game sounds (`sound`), the boards, pieces and sounds
//! embedded (`assets`), the board pages' own widgets (`widgets`), the engine's evaluation as
//! they show it (`engine_view`), pictures of a game (`game_image`), the puzzle database
//! (`puzzle_db`) and the game databases (`game_db`), the ChatGPT explainer (`ai`), a player's rating history (`rating_history`), and
//! moves said aloud (`voice`) through the microphone (`microphone`), the puzzle board
//! (`puzzle_board`), exporting games (`export`) and finding players (`player_search`).
//! It depends on the design system (`kchess-ui`) and the app's services (`kchess-app`), never on
//! the app: what it needs from it (whether voice owns Space) the app passes in.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

pub mod ai;
pub mod analysis;
pub mod assets;
pub mod board;
pub mod engine_view;
pub mod export;
pub mod game;
pub mod game_db;
pub mod game_image;
pub mod live_game;
pub mod microphone;
pub mod notation;
pub mod picture;
pub mod player_search;
pub mod position_engine;
pub mod puzzle_board;
pub mod puzzle_db;
pub mod rating_history;
pub mod room;
pub mod sound;
#[cfg(any(test, feature = "test-support"))]
// Test code, which other crates' tests build too.
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod testing;
pub mod voice;
pub mod widgets;

use gpui_kit::{App, Global};
// The design system and the app's services, under the names the app's modules use.
use kchess_app::{
    accounts, core, loading, log, network, played, prefs, quit, settings, slow_calls,
};
use kchess_ui::keys::keys;
use kchess_ui::{chart, dates, theme};

gpui_kit::actions!(kchess, [ToggleZen, FlipBoard]);

/// "Flip board", with the key that turns the board.
pub fn flip_label(cx: &App) -> String {
    match keys(&FlipBoard, cx) {
        Some(key) => format!("Flip board ({key})"),
        None => "Flip board".into(),
    }
}

/// Whether Space is the voice's while a board has the keys (held for push to talk): set by the
/// app, which owns the voice. Until it is set, Space is the board's.
#[derive(Clone, Copy)]
pub struct SpaceTalks(pub fn(&App) -> bool);

impl Global for SpaceTalks {}

/// Whether Space is the voice's now.
pub(crate) fn space_talks(cx: &App) -> bool {
    cx.try_global::<SpaceTalks>()
        .is_some_and(|SpaceTalks(talks)| talks(cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Space is the board's until the app says the voice holds it, and then only while it does.
    #[gpui_kit::test]
    fn space_is_the_voices_only_while_the_app_says_so(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            assert!(!space_talks(cx));
            cx.set_global(SpaceTalks(|_| true));
            assert!(space_talks(cx));
            cx.set_global(SpaceTalks(|_| false));
            assert!(!space_talks(cx));
        });
    }
}
