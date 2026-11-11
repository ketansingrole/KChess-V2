//! Quitting: the saves every open surface registers (`on_quit`) finish before the core closes,
//! and the work for after both (an installer that replaces the app, `after_quit`) runs only once
//! they have.

use std::sync::Arc;

use gpui_kit::{App, Global, PromptLevel};
use kchess_contracts::Level;

use crate::core::AppCore;

/// Saves to finish before the core closes (`on_quit`): each returns the work still to wait for.
#[derive(Default)]
pub struct QuitFlushes(Vec<Flush>);

type Flush = Box<dyn Fn(&mut App) -> Option<BoxedSave>>;

/// Whether a save reached the profile, or why it did not.
pub type Saved = Result<(), String>;

pub type BoxedSave = std::pin::Pin<Box<dyn Future<Output = Saved>>>;

impl Global for QuitFlushes {}

/// Run `flush` when the app quits, before the core closes, so unsaved work reaches the profile.
pub fn on_quit(cx: &mut App, flush: impl Fn(&mut App) -> Option<BoxedSave> + 'static) {
    cx.default_global::<QuitFlushes>().0.push(Box::new(flush));
}

/// Wait for every save, and report each that failed.
pub fn all_saved(saves: Vec<BoxedSave>) -> BoxedSave {
    Box::pin(async move {
        let failures: Vec<String> = futures::future::join_all(saves)
            .await
            .into_iter()
            .filter_map(Result::err)
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    })
}

/// Work for once the saves are done and the core has closed (`after_quit`).
#[derive(Default)]
pub struct QuitLast(Vec<Last>);

type Last = Box<dyn Fn(&mut App) -> Option<Box<dyn FnOnce()>>>;

impl Global for QuitLast {}

/// Run the work `last` returns (decided as the app starts quitting) only after every `on_quit`
/// save has finished and the core has closed: work that must not overlap them, such as an
/// installer that closes the app. Saves that fail or do not finish in time never run it, unless
/// the person chose to go on without them (`prepare_quit`).
pub fn after_quit(cx: &mut App, last: impl Fn(&mut App) -> Option<Box<dyn FnOnce()>> + 'static) {
    cx.default_global::<QuitLast>().0.push(Box::new(last));
}

/// How far preparing to quit has got.
#[derive(Default)]
struct Quitting {
    /// Saving with the window open: a second request waits for this one.
    preparing: bool,
    /// The person chose to quit without the saves that failed.
    discarded: bool,
    /// Everything was saved just now and the app is quitting straight after (`quit_saved`).
    saved: bool,
}

impl Global for Quitting {}

/// Every registered save, started now.
fn flush_all(cx: &mut App) -> Vec<BoxedSave> {
    let flushes = std::mem::take(&mut cx.default_global::<QuitFlushes>().0);
    let pending = flushes.iter().filter_map(|flush| flush(cx)).collect();
    // Surfaces opened while saving registered theirs meanwhile: keep both.
    let mut kept = flushes;
    kept.append(&mut cx.default_global::<QuitFlushes>().0);
    cx.default_global::<QuitFlushes>().0 = kept;
    pending
}

/// How long saving before quitting may take before the person is asked what to do.
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// What became of saves waited for until a deadline.
enum Waited {
    Saved(Saved),
    /// Still unanswered at the deadline: kept, so a retry waits for the same saves rather than
    /// starting them again over writes that may still land.
    Unanswered(BoxedSave),
}

/// Wait for `save` until `deadline` fires.
async fn wait_saved(save: BoxedSave, deadline: impl Future<Output = ()>) -> Waited {
    let deadline = std::pin::pin!(deadline);
    match futures::future::select(save, deadline).await {
        futures::future::Either::Left((saved, _)) => Waited::Saved(saved),
        futures::future::Either::Right(((), save)) => Waited::Unanswered(save),
    }
}

/// Save everything while the window is still open, then run `then` (quit, or install and
/// restart). A save that fails, or does not finish within `SAVE_WAIT`, asks first: retry, go on
/// without it (`without`, the button's label), or cancel and keep working (`cancelled`).
pub fn prepare_quit(
    cx: &mut App,
    without: &'static str,
    then: impl FnOnce(&mut App) + 'static,
    cancelled: impl FnOnce(&mut App) + 'static,
) {
    if std::mem::replace(&mut cx.default_global::<Quitting>().preparing, true) {
        return cx.defer(cancelled);
    }
    // A fresh decision: going on without the saves once (an update that then failed to install
    // and left the app open) does not excuse the saves of a later quit.
    cx.default_global::<Quitting>().discarded = false;
    cx.spawn(async move |cx| {
        let mut unanswered: Option<BoxedSave> = None;
        loop {
            let save = match unanswered.take() {
                Some(save) => save,
                None => all_saved(cx.update(flush_all)),
            };
            let deadline = cx.background_executor().timer(SAVE_WAIT);
            let saved = match wait_saved(save, deadline).await {
                Waited::Saved(saved) => saved,
                Waited::Unanswered(save) => {
                    unanswered = Some(save);
                    Err("Saving is taking longer than expected".to_string())
                }
            };
            let Err(cause) = saved else {
                cx.update(|cx| {
                    cx.default_global::<Quitting>().preparing = false;
                    then(cx);
                });
                return;
            };
            crate::log::write(
                Level::Warn,
                "app",
                &format!("Saving before quitting failed: {cause}"),
            );
            let answer = cx.update(|cx| {
                let window = cx
                    .active_window()
                    .or_else(|| cx.windows().first().copied())?;
                window
                    .update(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Warning,
                            "Some changes could not be saved",
                            Some(&cause),
                            &["Retry", without, "Cancel"],
                            cx,
                        )
                    })
                    .ok()
            });
            // No window to ask in: quit, but nothing installs over the unsaved work.
            let asked = answer.is_some();
            let choice = match answer {
                Some(answer) => answer.await.unwrap_or(2),
                None => 1,
            };
            match choice {
                0 => continue,
                1 => {
                    cx.update(|cx| {
                        let quitting = cx.default_global::<Quitting>();
                        quitting.preparing = false;
                        quitting.discarded = asked;
                        then(cx);
                    });
                    return;
                }
                _ => {
                    cx.update(|cx| {
                        cx.default_global::<Quitting>().preparing = false;
                        cancelled(cx);
                    });
                    return;
                }
            }
        }
    })
    .detach();
}

/// Quit, from a `prepare_quit` that saved everything: the quit handler does not save it all again.
pub fn quit_saved(cx: &mut App) {
    cx.default_global::<Quitting>().saved = true;
    cx.quit();
}

/// Whether the quit handler saves again. GPUI gives it 200 ms (`SHUTDOWN_TIMEOUT`) and drops it
/// unfinished after that, so saving everything a second time, a round trip to the core each,
/// could keep the core's close and the work after it (an installer) from ever running.
fn saves_again(saved: bool, discarded: bool) -> bool {
    !saved || discarded
}

/// Every registered flush, then the core's close, then the work for after both.
pub fn quit(cx: &mut App) -> impl Future<Output = ()> + 'static + use<> {
    let quitting = cx.default_global::<Quitting>();
    let (saved, discarded) = (std::mem::take(&mut quitting.saved), quitting.discarded);
    let pending = if saves_again(saved, discarded) {
        flush_all(cx)
    } else {
        Vec::new()
    };
    let last = std::mem::take(&mut cx.default_global::<QuitLast>().0);
    let last: Vec<Box<dyn FnOnce()>> = last.iter().filter_map(|last| last(cx)).collect();
    let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
    finish(pending, move || bridge.close(), last, discarded)
}

/// The saves, then `close`, then the work for after both: never before every save has finished,
/// and only when every save succeeded or the person chose to go on without them (`discarded`).
async fn finish(
    pending: Vec<BoxedSave>,
    close: impl FnOnce(),
    last: Vec<Box<dyn FnOnce()>>,
    discarded: bool,
) {
    let saved = all_saved(pending).await;
    close();
    if let Err(cause) = saved
        && !discarded
    {
        crate::log::write(
            Level::Warn,
            "app",
            &format!("Unsaved work kept the update from installing: {cause}"),
        );
        return;
    }
    for work in last {
        work();
    }
}

#[cfg(test)]
mod tests {
    use super::{BoxedSave, Waited, all_saved, finish, saves_again, wait_saved};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    #[test]
    fn an_unanswered_save_is_reported_at_the_deadline_and_kept_for_a_retry() {
        let mut cx = Context::from_waker(Waker::noop());
        let (answered, answer) = futures::channel::oneshot::channel::<()>();
        // The core received the save and never replies, with its connection still open.
        let save: BoxedSave = Box::pin(async move {
            let _ = answer.await;
            Ok(())
        });
        let mut waiting = Box::pin(wait_saved(save, std::future::ready(())));
        let Poll::Ready(Waited::Unanswered(save)) = waiting.as_mut().poll(&mut cx) else {
            panic!("an unanswered save must end the wait at the deadline");
        };
        // Retrying waits for the same save, which may still land, rather than sending it again.
        let mut retry = Box::pin(wait_saved(save, std::future::pending()));
        assert!(retry.as_mut().poll(&mut cx).is_pending());
        let _ = answered.send(());
        assert!(matches!(
            retry.as_mut().poll(&mut cx),
            Poll::Ready(Waited::Saved(Ok(())))
        ));
    }

    #[test]
    fn work_after_quitting_waits_for_every_save_and_the_core() {
        let order = Rc::new(RefCell::new(Vec::new()));
        let (saved, save) = futures::channel::oneshot::channel::<()>();
        let pending: Vec<BoxedSave> = vec![Box::pin(async move {
            let _ = save.await;
            Ok(())
        })];
        let closed = Rc::clone(&order);
        let installed = Rc::clone(&order);
        let mut quitting = Box::pin(finish(
            pending,
            move || closed.borrow_mut().push("close"),
            vec![Box::new(move || installed.borrow_mut().push("install"))],
            false,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        // A slow save: nothing installs while it is still being written.
        assert!(quitting.as_mut().poll(&mut cx).is_pending());
        assert!(order.borrow().is_empty());
        let _ = saved.send(());
        assert_eq!(quitting.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(*order.borrow(), ["close", "install"]);
    }

    /// A study and a session that both fail to save: the core still closes, but no installer
    /// runs over the unsaved work unless the person chose to go on without it.
    #[test]
    fn a_failed_save_keeps_the_update_from_installing() {
        for (discarded, expected) in [(false, vec!["close"]), (true, vec!["close", "install"])] {
            let order = Rc::new(RefCell::new(Vec::new()));
            let pending: Vec<BoxedSave> = vec![
                Box::pin(async { Err("study: disk full".to_string()) }),
                Box::pin(async { Err("session: disk full".to_string()) }),
            ];
            let closed = Rc::clone(&order);
            let installed = Rc::clone(&order);
            let mut quitting = Box::pin(finish(
                pending,
                move || closed.borrow_mut().push("close"),
                vec![Box::new(move || installed.borrow_mut().push("install"))],
                discarded,
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert_eq!(quitting.as_mut().poll(&mut cx), Poll::Ready(()));
            assert_eq!(*order.borrow(), expected);
        }
    }

    /// "Restart to update" saved everything, then quit; the quit handler saved it all again, and
    /// when that took over GPUI's 200 ms the handler was dropped before the installer started.
    /// "Restart without saving" once, then an update that failed to install, left the app open
    /// with the choice kept: a later quit installed over work it could not save.
    #[gpui_kit::test]
    fn quitting_without_the_saves_is_chosen_again_for_each_quit(cx: &mut gpui_kit::TestAppContext) {
        let mut app = crate::e2e::Harness::start(cx);
        app.with_window(|_, cx| {
            cx.default_global::<super::Quitting>().discarded = true;
            super::prepare_quit(cx, "Quit without saving", |_| {}, |_| {});
        });
        let discarded = app.with_window(|_, cx| cx.default_global::<super::Quitting>().discarded);
        assert!(!discarded);
    }

    #[test]
    fn a_quit_straight_after_saving_does_not_save_again() {
        assert!(
            !saves_again(true, false),
            "just saved: nothing to save again"
        );
        assert!(
            saves_again(false, false),
            "a quit that did not save first saves now"
        );
        assert!(saves_again(true, true), "failed saves are tried once more");
        // Nothing left to save: the core closes and the installer runs in the same poll.
        let order = Rc::new(RefCell::new(Vec::new()));
        let (closed, installed) = (Rc::clone(&order), Rc::clone(&order));
        let mut quitting = Box::pin(finish(
            Vec::new(),
            move || closed.borrow_mut().push("close"),
            vec![Box::new(move || installed.borrow_mut().push("install"))],
            false,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(quitting.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(*order.borrow(), ["close", "install"]);
    }

    #[test]
    fn every_failure_is_reported() {
        let saves: Vec<BoxedSave> = vec![
            Box::pin(async { Ok(()) }),
            Box::pin(async { Err("study".to_string()) }),
            Box::pin(async { Err("session".to_string()) }),
        ];
        let mut all = all_saved(saves);
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(
            all.as_mut().poll(&mut cx),
            Poll::Ready(Err("study\nsession".to_string()))
        );
    }
}
