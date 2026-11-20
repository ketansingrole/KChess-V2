//! What the board and the pages know of the live Lichess game the app plays
//! (`apps/gpui/src/online.rs`): whether engine help may show, and whether a game holds the
//! connection. The app keeps it up to date; a page reads it, or observes it to stop its engine
//! as a game starts.

use gpui_kit::{App, AppContext as _, Context, Entity, Global};

pub struct LiveGame {
    assistance_allowed: bool,
    busy_with_game: bool,
}

impl LiveGame {
    /// The engine may help: no live game, and none still being looked for.
    pub fn assistance_allowed(&self) -> bool {
        self.assistance_allowed
    }

    /// A game of the account's is using the connection.
    pub fn busy_with_game(&self) -> bool {
        self.busy_with_game
    }

    /// The live game's state now; observers hear only of a change.
    pub fn set(&mut self, assistance_allowed: bool, busy_with_game: bool, cx: &mut Context<Self>) {
        if (self.assistance_allowed, self.busy_with_game) != (assistance_allowed, busy_with_game) {
            self.assistance_allowed = assistance_allowed;
            self.busy_with_game = busy_with_game;
            cx.notify();
        }
    }
}

/// How views reach it: `cx.global::<LiveGameHandle>().0`, or `handle(cx)`.
#[derive(Clone)]
pub struct LiveGameHandle(pub Entity<LiveGame>);

impl Global for LiveGameHandle {}

pub fn handle(cx: &App) -> Entity<LiveGame> {
    cx.global::<LiveGameHandle>().0.clone()
}

/// No help until the app's game has checked Lichess for a game in progress, as it starts.
pub fn init(cx: &mut App) {
    let live = cx.new(|_| LiveGame {
        assistance_allowed: false,
        busy_with_game: false,
    });
    cx.set_global(LiveGameHandle(live));
}

#[cfg(test)]
mod tests {
    use gpui_kit::AppContext as _;

    /// Help is held back until the game says otherwise, and observers hear of a change only.
    #[gpui_kit::test]
    fn observers_hear_only_of_a_change(cx: &mut gpui_kit::TestAppContext) {
        cx.update(super::init);
        let live = cx.update(|cx| super::handle(cx));
        let heard = std::rc::Rc::new(std::cell::Cell::new(0));
        let _watch = cx.update(|cx| {
            let heard = heard.clone();
            cx.observe(&live, move |_, _| heard.set(heard.get() + 1))
        });
        live.read_with(cx, |live, _| {
            assert!(!live.assistance_allowed() && !live.busy_with_game());
        });
        live.update(cx, |live, cx| live.set(true, false, cx));
        cx.run_until_parked();
        assert_eq!(heard.get(), 1);
        live.update(cx, |live, cx| live.set(true, false, cx));
        cx.run_until_parked();
        assert_eq!(heard.get(), 1, "the same state again tells no one");
        live.update(cx, |live, cx| live.set(false, true, cx));
        cx.run_until_parked();
        assert_eq!(heard.get(), 2);
        live.read_with(cx, |live, _| {
            assert!(!live.assistance_allowed() && live.busy_with_game());
        });
    }
}
