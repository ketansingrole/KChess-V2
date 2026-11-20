//! The core as the GPUI app holds it (`kchess_service::CoreHandle`): the profile's shared core
//! service (`docs/shared-core.md`), which every KChess window uses, or a core embedded in the app.
//! Either runs on a Tokio runtime; the UI awaits its answers through channels, and its events
//! reach views through `CoreEvents`, an entity they subscribe to. The app opens it and answers
//! its host requests (`apps/gpui/src/core.rs`).

use crate::log::ResultExt as _;
use std::sync::Arc;

use gpui_kit::{App, Entity, EventEmitter, Global, Subscription};
use kchess_contracts::calls::CoreCall;
use kchess_contracts::types::ProfileArea;
use kchess_contracts::{CoreError, HostReply};
use kchess_service::CoreHandle;

pub use kchess_contracts::CoreEvent;
pub use kchess_service::ClientEvent;

/// What views subscribe to for core events, and for `ClientEvent::Reconnected`.
pub struct CoreEvents;

impl EventEmitter<ClientEvent> for CoreEvents {}

/// The core and the runtime it runs on.
pub struct CoreBridge {
    runtime: tokio::runtime::Runtime,
    handle: CoreHandle,
}

impl CoreBridge {
    /// The core behind `handle`, running on `runtime`.
    pub fn new(runtime: tokio::runtime::Runtime, handle: CoreHandle) -> CoreBridge {
        CoreBridge { runtime, handle }
    }

    /// This app's connection on the core: `origin` of its own writes in `profile:changed`.
    pub fn connection(&self) -> u64 {
        self.handle.connection()
    }

    /// Tells the shared core whether this app's window is focused: the window focused last
    /// raises system notifications.
    pub fn focus(&self, focused: bool) {
        self.handle.focus(focused);
    }

    /// Runs `work` on the core's runtime; the future resolves on any executor.
    fn spawn<T: Send + 'static, W: Future<Output = Result<T, CoreError>> + Send + 'static>(
        &self,
        work: W,
    ) -> impl Future<Output = Result<T, CoreError>> + 'static + use<T, W> {
        let task = self.runtime.spawn(work);
        async move {
            task.await
                .unwrap_or_else(|_| Err(CoreError::new("The KChess core stopped")))
        }
    }

    /// `request` for a lookup a newer one replaces: dropped before its answer, it cancels the
    /// call in the core rather than leaving it to run for nobody.
    pub fn lookup<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        abortable(self.runtime.handle(), self.handle.request(call))
    }

    /// A typed core call (`kchess_contracts::calls`); the future resolves on any executor.
    pub fn request<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        self.spawn(self.handle.request(call))
    }

    /// A typed core call waited for: only before the window opens, as the app needs its library
    /// and preferences before it draws.
    #[track_caller]
    pub fn request_now<C: CoreCall + Send + 'static>(&self, call: C) -> Result<C::Answer, CoreError>
    where
        C::Answer: Send,
    {
        crate::slow_calls::call("a core call waited for (AppCore::request_now)");
        let answer = self.request(call);
        self.runtime.block_on(answer)
    }

    /// The answer to the host request `id`.
    pub fn reply(
        &self,
        id: u64,
        reply: HostReply,
    ) -> impl Future<Output = Result<(), CoreError>> + use<> {
        self.spawn(self.handle.reply(id, reply))
    }

    /// Done with the core: an embedded one cancels its work and releases the profile, waiting at
    /// most a few seconds; the shared core closes on its own once no window uses it.
    #[track_caller]
    pub fn close(&self) {
        crate::slow_calls::call("the core closing (AppCore::close)");
        let handle = self.handle.clone();
        let closing = self.runtime.spawn(async move { handle.close().await });
        self.runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), closing).await
            })
            .map_err(|_| "The core did not close within 5 s")
            .warn_on_err();
    }
}

/// Runs `work` on `runtime`; the future resolves on any executor, and dropped first it aborts
/// the work, which drops the core call with it.
fn abortable<T: Send + 'static, W: Future<Output = Result<T, CoreError>> + Send + 'static>(
    runtime: &tokio::runtime::Handle,
    work: W,
) -> impl Future<Output = Result<T, CoreError>> + 'static + use<T, W> {
    struct Abort(tokio::task::AbortHandle);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let task = runtime.spawn(work);
    let abort = Abort(task.abort_handle());
    async move {
        let _abort = abort;
        task.await
            .unwrap_or_else(|_| Err(CoreError::new("The KChess core stopped")))
    }
}

/// Orders a surface's reads of the profile : only the newest
/// read is shown, and one started before a local change is read again rather than shown over it.
#[derive(Default)]
pub struct Reads {
    latest: u64,
    edits: u64,
}

/// A read in flight: which one it was, and the local changes made before it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    id: u64,
    edits: u64,
}

/// What to do with a read's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Show,
    /// A newer read was started: drop this one.
    Superseded,
    /// A local change came after the read started: read again.
    Again,
}

impl Reads {
    pub fn start(&mut self) -> Read {
        self.latest += 1;
        Read {
            id: self.latest,
            edits: self.edits,
        }
    }

    /// A local change was made (sent to the core, or shown before it is).
    pub fn edited(&mut self) {
        self.edits += 1;
    }

    pub fn answer(&self, read: Read) -> Answer {
        if read.id != self.latest {
            Answer::Superseded
        } else if read.edits != self.edits {
            Answer::Again
        } else {
            Answer::Show
        }
    }
}

/// Runs `reload` when another window changes one of `areas` of the profile (`profile:changed`,
/// `docs/shared-core.md`), and after reconnecting to the core, which did not report what changed
/// meanwhile; this app's own writes are ignored. For as long as the app runs; a page keeps
/// `profile_changes` with its other subscriptions.
pub fn on_profile_change(
    cx: &mut App,
    areas: &'static [ProfileArea],
    reload: impl Fn(&mut App) + 'static,
) {
    profile_changes(cx, areas, reload).detach();
}

/// `on_profile_change` while the subscription is kept.
pub fn profile_changes(
    cx: &mut App,
    areas: &'static [ProfileArea],
    reload: impl Fn(&mut App) + 'static,
) -> Subscription {
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, move |_, event: &ClientEvent, cx| {
        let change = match event {
            ClientEvent::Reconnected => return reload(cx),
            ClientEvent::Event(CoreEvent::ProfileChanged(change)) => change,
            _ => return,
        };
        if change.origin == cx.global::<AppCore>().bridge.connection() {
            return;
        }
        if change.areas.iter().any(|area| areas.contains(area)) {
            reload(cx);
        }
    })
}

/// Which connection raises system notifications now (`alerts:owner`); `None` until the core says.
#[derive(Default)]
pub struct AlertsOwnerGlobal(pub Option<u64>);

impl Global for AlertsOwnerGlobal {}

/// Whether this app may raise a system notification: it is the alert owner, or nobody is.
pub fn alerts_here(cx: &App) -> bool {
    let own = cx.global::<AppCore>().bridge.connection();
    cx.try_global::<AlertsOwnerGlobal>()
        .and_then(|owner| owner.0)
        .is_none_or(|owner| owner == own)
}

/// The app's core, reachable from any view: `cx.global::<AppCore>()`.
#[derive(Clone)]
pub struct AppCore {
    pub bridge: Arc<CoreBridge>,
    pub events: Entity<CoreEvents>,
}

impl Global for AppCore {}

impl AppCore {
    /// Run a typed core call (`kchess_contracts::calls`): its method with its arguments, and its
    /// answer read in its shape.
    pub fn request<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        self.bridge.request(call)
    }

    /// `request` for a lookup a newer one replaces (`CoreBridge::lookup`).
    pub fn lookup<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        self.bridge.lookup(call)
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, Reads};
    use kchess_contracts::types::ProfileArea;

    /// Closing an embedded core ends it: what is asked of it afterwards fails.
    #[gpui_kit::test]
    fn a_closed_core_answers_nothing_more(cx: &mut gpui_kit::TestAppContext) {
        let _core = cx.update(crate::tests::start_core);
        let core = cx.read(|cx| cx.global::<super::AppCore>().clone());
        assert!(
            core.bridge
                .request_now(kchess_contracts::calls::Usage {})
                .is_ok()
        );
        core.bridge.close();
        assert!(
            core.bridge
                .request_now(kchess_contracts::calls::Usage {})
                .is_err()
        );
    }

    /// 2026-11-15: a cloud evaluation the next position replaced kept running in the core, since
    /// the app's call ran detached on the core's runtime; dropping it now ends the work.
    #[test]
    fn a_dropped_lookup_stops_its_work() {
        struct Dropped(std::sync::mpsc::Sender<()>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (sender, dropped) = std::sync::mpsc::channel();
        let (started, running) = std::sync::mpsc::channel();
        let lookup = super::abortable(runtime.handle(), async move {
            let _work = Dropped(sender);
            let _ = started.send(());
            std::future::pending::<Result<(), kchess_contracts::CoreError>>().await
        });
        running.recv().unwrap();
        drop(lookup);
        assert!(
            dropped
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok(),
            "the work went on after its lookup was dropped"
        );
    }

    #[test]
    fn only_the_newest_read_without_later_changes_is_shown() {
        let mut reads = Reads::default();
        let older = reads.start();
        let newer = reads.start();
        // Answers in reverse order: the older one never replaces the newer.
        assert_eq!(reads.answer(newer), Answer::Show);
        assert_eq!(reads.answer(older), Answer::Superseded);
        // A local change after a read started: that read may predate it.
        let read = reads.start();
        reads.edited();
        assert_eq!(reads.answer(read), Answer::Again);
        let again = reads.start();
        assert_eq!(reads.answer(again), Answer::Show);
    }

    /// What reads each profile area again when another window writes it, or why nothing does.
    /// A new area does not compile here until that is decided.
    fn observed_by(area: ProfileArea) -> &'static str {
        match area {
            ProfileArea::Studies => "studies.rs",
            ProfileArea::PlayedGames => "played.rs",
            ProfileArea::Drills => "pages/practice/mistakes.rs",
            ProfileArea::Sessions => "pages/home: the game to continue",
            ProfileArea::Tournaments => "pages/tournaments: the events joined",
            ProfileArea::Repertoire => {
                "ignored: the repertoire trainer reads it each time it is shown, and a reload \
                 would replace the line being practised"
            }
            ProfileArea::LichessGames => "accounts.rs: every write of it writes Accounts too",
            ProfileArea::Accounts | ProfileArea::Usage => "accounts.rs",
            ProfileArea::DesktopSettings => "settings.rs: the SettingsSaved event",
            ProfileArea::Puzzles => "puzzle_db.rs",
            ProfileArea::VoiceHistory | ProfileArea::Engine => "pages/settings",
            ProfileArea::ViewPreferences => "prefs.rs",
        }
    }

    #[test]
    fn every_write_of_lichess_games_also_writes_accounts() {
        for method in kchess_contracts::core_api::METHODS {
            if method.writes.contains(&ProfileArea::LichessGames) {
                assert!(
                    method.writes.contains(&ProfileArea::Accounts),
                    "{} writes Lichess games alone: {}",
                    method.name,
                    observed_by(ProfileArea::LichessGames)
                );
            }
        }
    }
}
