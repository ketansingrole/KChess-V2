//! What the board and the pages know of the live Lichess game the app plays
//! (`apps/gpui/src/online.rs`): whether engine help may show, and whether a game holds the
//! connection. The app keeps it up to date; a page reads it, or observes it to stop its engine
//! as a game starts, and asks again what the core refused while help was held back (`Refused`).

use gpui_kit::{App, AppContext as _, Context, Entity, Global, Subscription};
use kchess_contracts::{CoreError, ErrorKind};

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

/// Whether a page's latest request was refused while help was held back (`ErrorKind::Withheld`),
/// so it is asked again once help is lent. A page opened as the app starts asks before Lichess has
/// answered whether a game is in progress: the Database page kept that refusal until a filter
/// changed (2026-11-25).
#[derive(Debug, Default)]
pub struct Refused {
    /// The latest request was refused while help was held back.
    withheld: bool,
    /// The latest request is the one made again as its refusal came back after help was lent:
    /// refused too, it waits for help to change rather than asking at once again.
    made_again: bool,
}

impl Refused {
    /// Watches help for the view whose `Refused` `slot` finds: once lent, a refused request is
    /// made again by `again`. Told by the refusal, not by seeing help switch off and on: both can
    /// land in one notification, or while the request is still on its way. Kept with the view's
    /// subscriptions.
    pub fn watch<V: 'static>(
        cx: &mut Context<V>,
        slot: fn(&mut V) -> &mut Refused,
        again: fn(&mut V, &mut Context<V>),
    ) -> Subscription {
        cx.observe(&handle(cx), move |view, live, cx| {
            if live.read(cx).assistance_allowed() && slot(view).withheld {
                again(view, cx);
            }
        })
    }

    /// A request goes out, or none will: an earlier refusal no longer counts.
    pub fn asking(&mut self) {
        self.withheld = false;
        self.made_again = false;
    }

    /// The latest request's answer. Refused while help was held back, it is asked again once help
    /// is lent; if help is lent already (lent while the refusal was on its way, so no change of
    /// help is left to answer it), at once, but only once. Asked after this update, as `again`
    /// may replace the task that answered.
    pub fn answered<V: 'static, T>(
        &mut self,
        answer: &Result<T, CoreError>,
        slot: fn(&mut V) -> &mut Refused,
        again: fn(&mut V, &mut Context<V>),
        cx: &mut Context<V>,
    ) {
        self.withheld = matches!(answer, Err(cause) if cause.kind == Some(ErrorKind::Withheld));
        let lent = handle(cx).read(cx).assistance_allowed();
        if !self.withheld || !lent || self.made_again {
            return;
        }
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            // Closed meanwhile, the view has nothing to ask.
            view.update(cx, |view, cx| {
                if slot(view).withheld {
                    again(view, cx);
                    slot(view).made_again = true;
                }
            })
            .ok();
        });
    }

    /// The latest request was refused while help was held back.
    pub fn withheld(&self) -> bool {
        self.withheld
    }
}

#[cfg(test)]
mod tests {
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

    use super::Refused;
    use gpui_kit::{AppContext as _, Context, Subscription};
    use kchess_contracts::{CoreError, ErrorKind};

    /// A page whose requests count how often they were made.
    struct Page {
        refused: Refused,
        asked: usize,
        _watch: Subscription,
    }

    fn ask(page: &mut Page, _: &mut Context<Page>) {
        page.refused.asking();
        page.asked += 1;
    }

    fn page(cx: &mut gpui_kit::TestAppContext) -> gpui_kit::Entity<Page> {
        cx.update(super::init);
        cx.new(|cx| Page {
            refused: Refused::default(),
            asked: 0,
            _watch: Refused::watch(cx, |page| &mut page.refused, ask),
        })
    }

    fn answer(
        page: &gpui_kit::Entity<Page>,
        answer: Result<(), CoreError>,
        cx: &mut gpui_kit::TestAppContext,
    ) {
        page.update(cx, |page, cx| {
            page.refused
                .answered(&answer, |page| &mut page.refused, ask, cx)
        });
        cx.run_until_parked();
    }

    fn lend(lent: bool, cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| super::handle(cx).update(cx, |live, cx| live.set(lent, false, cx)));
        cx.run_until_parked();
    }

    fn withheld() -> Result<(), CoreError> {
        Err(CoreError::new("Held back.").with_kind(ErrorKind::Withheld))
    }

    fn asked(page: &gpui_kit::Entity<Page>, cx: &mut gpui_kit::TestAppContext) -> usize {
        page.read_with(cx, |page, _| page.asked)
    }

    /// 2026-11-25: the Database page, opened as the app started, was refused its games before
    /// Lichess had answered whether a game was in progress, and kept the refusal once help was
    /// lent. Refused while help is held back, a request is made again as help is lent.
    #[gpui_kit::test]
    fn a_refusal_is_asked_again_once_help_is_lent(cx: &mut gpui_kit::TestAppContext) {
        let page = page(cx);
        answer(&page, withheld(), cx);
        assert_eq!(asked(&page, cx), 0, "help is still held back");
        lend(true, cx);
        assert_eq!(asked(&page, cx), 1);
        answer(&page, Ok(()), cx);
        lend(false, cx);
        lend(true, cx);
        assert_eq!(asked(&page, cx), 1, "an answer is not asked again");
    }

    /// Lent while the refusal was on its way, no change of help is left to answer it: it is
    /// asked again at once, but once only, and then waits for help to change.
    #[gpui_kit::test]
    fn a_refusal_after_help_was_lent_is_asked_again_once(cx: &mut gpui_kit::TestAppContext) {
        let page = page(cx);
        lend(true, cx);
        answer(&page, withheld(), cx);
        assert_eq!(asked(&page, cx), 1);
        answer(&page, withheld(), cx);
        assert_eq!(
            asked(&page, cx),
            1,
            "refused again, it waits for help to change"
        );
        lend(false, cx);
        lend(true, cx);
        assert_eq!(asked(&page, cx), 2);
    }

    /// Any other failure waits for the page to ask again, and so does a refusal followed by
    /// another request.
    #[gpui_kit::test]
    fn only_the_latest_refusal_is_asked_again(cx: &mut gpui_kit::TestAppContext) {
        let page = page(cx);
        answer(&page, Err(CoreError::new("Lookup failed.")), cx);
        lend(true, cx);
        assert_eq!(asked(&page, cx), 0);
        lend(false, cx);
        answer(&page, withheld(), cx);
        page.update(cx, |page, _| page.refused.asking());
        lend(true, cx);
        assert_eq!(asked(&page, cx), 0);
    }
}
