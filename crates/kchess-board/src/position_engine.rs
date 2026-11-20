//! One visible position's engine search, for boards that are not the
//! analysis board: a new request replaces the search running (the core cancels it when the next
//! starts), and only updates for the request asked last are kept.
//!
//! Every search and stop names its client id, which rises with each request, so the core can
//! tell a late one: it runs calls in any order, and a stop or start that lands after a newer
//! search of the window has started changes nothing (`docs/shared-core.md`, Analysis).

use kchess_contracts::calls;
use kchess_contracts::types::{AnalysisRequest, AnalysisUpdate, EngineLine};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::*;
use kchess_contracts::{CoreError, Level};

use crate::core::AppCore;
use crate::game::tree::analysis_context;

/// Client ids unique across every engine search in the app, the analysis board's included.
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(0);

pub fn next_client() -> u64 {
    let now = crate::dates::now_millis().max(0) as u64;
    let _ = NEXT_CLIENT.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
    NEXT_CLIENT.fetch_add(1, Ordering::Relaxed) + 1
}

/// The context an update must carry to belong to a request.
fn context_of(request: &AnalysisRequest) -> String {
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    analysis_context(root, request.moves.as_deref().unwrap_or_default())
}

/// Stop `client`'s search, the last one a view started (none: it started none, and has nothing to
/// stop); `release` lets the engine go too, where a pause keeps it warm. A newer search of the
/// window, from this view or another, carries on. A failure is only logged, under `source`.
pub fn stop_analysis(client: Option<u64>, release: bool, source: &'static str, cx: &mut App) {
    let Some(client) = client else {
        return;
    };
    let stop = cx.global::<AppCore>().request(calls::StopAnalysis {
        kill: release.then_some(true),
        client: Some(kchess_contracts::check::Bounded(client)),
    });
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = stop.await {
                crate::log::write(
                    Level::Warn,
                    source,
                    &format!("stopping analysis failed: {}", cause.message),
                );
            }
        })
        .detach();
}

/// What a search's failed start shows: nothing for a cancellation (another window took the
/// engine, or this one asked for something since), else why it failed.
pub fn start_error(cause: CoreError) -> Option<String> {
    (!cause.aborted).then_some(cause.message)
}

#[derive(Default)]
pub struct PositionEngine {
    /// The search asked for, or none.
    wanted: Option<AnalysisRequest>,
    context: String,
    client_id: u64,
    /// The client id of the last search sent to the core, which a stop names.
    started: Option<u64>,
    latest_id: u64,
    /// The latest update for the search asked for.
    pub update: Option<AnalysisUpdate>,
    pub error: Option<String>,
    pub busy: bool,
    _start: Option<Task<()>>,
}

impl PositionEngine {
    /// Search `request` (or nothing), replacing the search running; the same request again
    /// changes nothing. `engine` finds this engine in its owner.
    pub fn search<V: 'static>(
        &mut self,
        request: Option<AnalysisRequest>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        if request == self.wanted {
            return;
        }
        self.restart(request, engine, cx);
    }

    /// Search what was asked for again from the start (a retry, or after the core reconnected).
    pub fn retry<V: 'static>(
        &mut self,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        self.restart(self.wanted.clone(), engine, cx);
    }

    fn restart<V: 'static>(
        &mut self,
        request: Option<AnalysisRequest>,
        engine: fn(&mut V) -> &mut PositionEngine,
        cx: &mut Context<V>,
    ) {
        let id = next_client();
        self.client_id = id;
        self.latest_id = 0;
        self.context = request.as_ref().map(context_of).unwrap_or_default();
        self.wanted = request.clone();
        self.update = None;
        self.error = None;
        self.busy = request.is_some();
        let Some(mut request) = request else {
            // Cancellation reaches the core's search, not only its answers.
            self._start = None;
            stop_analysis(self.started, false, "position-engine", cx);
            return;
        };
        request.client_id = Some(id);
        self.started = Some(id);
        let core = cx.global::<AppCore>().clone();
        // Dropped unsent when a newer request replaces it.
        self._start = Some(cx.spawn(async move |this, cx| {
            let start = core.request(calls::StartAnalysis { request });
            let Err(cause) = start.await else {
                return;
            };
            let _ = this.update(cx, |owner, cx| {
                let engine = engine(owner);
                if engine.client_id != id {
                    return;
                }
                engine.busy = false;
                cx.notify();
                let Some(error) = start_error(cause) else {
                    return;
                };
                crate::log::write(
                    Level::Warn,
                    "position-engine",
                    &format!("starting analysis failed: {error}"),
                );
                engine.error = Some(error);
            });
        }));
    }

    /// An `engine:analysis` event: kept, and true, when it is for the search asked for.
    pub fn receive(&mut self, update: &AnalysisUpdate) -> bool {
        let Some(wanted) = &self.wanted else {
            return false;
        };
        if update.client_id != i64::try_from(self.client_id).ok()
            || wanted.fen != update.fen
            || update.context != self.context
            || update.id < self.latest_id
        {
            return false;
        }
        self.latest_id = update.id;
        self.error = update.error.clone();
        self.update = self.error.is_none().then(|| update.clone());
        self.busy = !update.done;
        true
    }

    /// The best line of the latest update.
    pub fn best(&self) -> Option<&EngineLine> {
        self.update.as_ref()?.lines.first()
    }

    pub fn lines(&self) -> &[EngineLine] {
        self.update
            .as_ref()
            .map_or(&[], |update| update.lines.as_slice())
    }
}

/// The longest an engine is waited for to start reporting on a look before the look ends anyway.
/// The core bounds an engine's start and answers a failed one with an error, so this only ends a
/// look whose engine went silent.
const LOOK_START: Duration = Duration::from_secs(20);

/// A look at a position that ends at a depth or after a while of the engine's own search: the
/// while counts from the engine's first report, not from the request. Counted from the request,
/// the time a loaded machine took to start the engine and hand it the position used up Guess the
/// move's two seconds, and a guess was judged with no engine lines (`d5` where the engine marks
/// `d5?!`, Windows CI, 2026-11-10 to 11-17). Same position, same verdict, however busy the machine.
#[derive(Default)]
pub struct EngineLook {
    ends: Option<Task<()>>,
    reported: bool,
    long: Duration,
}

impl EngineLook {
    /// Start a look lasting `long` of the engine's search; `done` ends it, once.
    pub fn start<V: 'static>(
        &mut self,
        long: Duration,
        done: fn(&mut V, &mut Context<V>),
        cx: &mut Context<V>,
    ) {
        self.long = long;
        self.reported = false;
        self.ends = Some(Self::end_after(LOOK_START, done, cx));
    }

    /// The engine reported on the look: its time starts now, at the first report.
    pub fn reported<V: 'static>(&mut self, done: fn(&mut V, &mut Context<V>), cx: &mut Context<V>) {
        if self.ends.is_some() && !self.reported {
            self.reported = true;
            self.ends = Some(Self::end_after(self.long, done, cx));
        }
    }

    /// No look running any more: its end is dropped unsent.
    pub fn clear(&mut self) {
        self.ends = None;
        self.reported = false;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn running(&self) -> bool {
        self.ends.is_some()
    }

    fn end_after<V: 'static>(
        after: Duration,
        done: fn(&mut V, &mut Context<V>),
        cx: &mut Context<V>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, done);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PositionEngine, next_client};
    use kchess_contracts::types::AnalysisRequest;
    use serde_json::json;

    /// A look's time counts from the engine's first report: a slow start does not use it up.
    #[gpui_kit::test]
    fn a_look_lasts_its_time_from_the_engines_first_report(cx: &mut gpui_kit::TestAppContext) {
        use super::{EngineLook, LOOK_START};
        use gpui_kit::AppContext as _;
        use std::time::Duration;

        #[derive(Default)]
        struct Owner {
            look: EngineLook,
            ended: u32,
        }
        fn done(owner: &mut Owner, _: &mut gpui_kit::Context<Owner>) {
            owner.ended += 1;
            owner.look.clear();
        }
        let owner = cx.new(|_| Owner::default());
        let ended =
            |cx: &mut gpui_kit::TestAppContext| cx.read_entity(&owner, |owner, _| owner.ended);
        let long = Duration::from_secs(2);
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.start(long, done, cx)));
        // The engine takes three seconds to start: longer than the look, not over yet.
        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();
        assert_eq!(ended(cx), 0, "a slow start does not end the look");
        assert!(cx.read_entity(&owner, |owner, _| owner.look.running()));
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.reported(done, cx)));
        cx.executor().advance_clock(long - Duration::from_millis(1));
        cx.run_until_parked();
        // A later report does not start the time again.
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.reported(done, cx)));
        assert_eq!(ended(cx), 0);
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(
            ended(cx),
            1,
            "the look ends its time after the first report"
        );
        assert!(!cx.read_entity(&owner, |owner, _| owner.look.running()));

        // A report after the look ended starts nothing.
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.reported(done, cx)));
        cx.executor().advance_clock(Duration::from_secs(60));
        cx.run_until_parked();
        assert_eq!(ended(cx), 1);

        // An engine that never reports: the look ends once it has waited for the start.
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.start(long, done, cx)));
        cx.executor()
            .advance_clock(LOOK_START - Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(ended(cx), 1);
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(ended(cx), 2, "a silent engine's look still ends");

        // A cleared look never ends.
        cx.update(|cx| owner.update(cx, |owner, cx| owner.look.start(long, done, cx)));
        cx.update(|cx| owner.update(cx, |owner, _| owner.look.clear()));
        cx.executor().advance_clock(LOOK_START);
        cx.run_until_parked();
        assert_eq!(ended(cx), 2);
    }

    /// A start the core cancelled is no failure to show; any other says why it failed.
    #[test]
    fn only_a_failed_start_shows_an_error() {
        use super::start_error;
        use kchess_contracts::CoreError;
        assert_eq!(
            start_error(CoreError::aborted("This operation was aborted")),
            None
        );
        assert_eq!(
            start_error(CoreError::new("Stockfish could not be found.")),
            Some("Stockfish could not be found.".to_string())
        );
    }

    /// A view's search reaches the core under its own client id, and a stop names the search it
    /// ends: the core refuses a start for that search that runs after the stop (the core runs
    /// calls in any order). The core refuses it before it looks for an engine, which the test's
    /// core has none of.
    #[gpui_kit::test]
    fn a_search_and_a_stop_name_their_client(cx: &mut gpui_kit::TestAppContext) {
        use super::{PositionEngine, stop_analysis};
        use crate::core::{AppCore, ClientEvent, CoreEvent};
        use gpui_kit::AppContext as _;
        use kchess_contracts::{CoreError, calls};
        use std::sync::{Arc, Mutex};

        struct Owner {
            engine: PositionEngine,
            _events: gpui_kit::Subscription,
        }
        fn engine_of(owner: &mut Owner) -> &mut PositionEngine {
            &mut owner.engine
        }
        let mut app = crate::testing::Started::new(cx);
        // The core refuses analysis until its startup check for a live Lichess game is over.
        app.wait_for("the startup check to finish", |_, cx| {
            crate::live_game::handle(cx).read(cx).assistance_allowed()
        });
        let owner = app.with_window(|_, cx| {
            let events = cx.global::<AppCore>().events.clone();
            cx.new(|cx| Owner {
                engine: PositionEngine::default(),
                _events: cx.subscribe(&events, |owner: &mut Owner, _, event: &ClientEvent, cx| {
                    if let ClientEvent::Event(CoreEvent::Analysis(update)) = event
                        && owner.engine.receive(update)
                    {
                        cx.notify();
                    }
                }),
            })
        });
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let request = AnalysisRequest {
            fen: start.to_string(),
            lines: 1,
            infinite: Some(true),
            root_fen: None,
            moves: Some(Vec::new()),
            client_id: None,
            variant: None,
        };
        app.with_window(|_, cx| {
            owner.update(cx, |owner, cx| {
                owner.engine.search(Some(request.clone()), engine_of, cx)
            })
        });
        let (started, client_id, busy) = app.with_window(|_, cx| {
            let engine = &owner.read(cx).engine;
            (engine.started, engine.client_id, engine.busy)
        });
        assert_eq!(started, Some(client_id), "the search is sent as its client");
        assert!(busy);

        // A stop for a search whose start the core has not run yet: that start is refused.
        let client = next_client();
        app.with_window(|_, cx| stop_analysis(Some(client), false, "test", cx));
        let refused: Arc<Mutex<Option<Result<u64, CoreError>>>> = Arc::default();
        let answer = Arc::clone(&refused);
        app.with_window(|_, cx| {
            let late = cx.global::<AppCore>().request(calls::StartAnalysis {
                request: AnalysisRequest {
                    client_id: Some(client),
                    ..request
                },
            });
            cx.background_executor()
                .spawn(async move {
                    *answer.lock().expect("answer") = Some(late.await);
                })
                .detach();
        });
        app.wait_for("the late start's answer", |_, _| {
            refused.lock().expect("answer").is_some()
        });
        let answer = refused.lock().expect("answer").take().expect("answered");
        assert!(
            answer.as_ref().is_err_and(|cause| cause.aborted),
            "{answer:?}"
        );
    }

    #[test]
    fn client_ids_never_repeat() {
        let first = next_client();
        assert!(next_client() > first);
    }

    #[test]
    fn only_the_search_asked_for_is_kept() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        let request = AnalysisRequest {
            fen: fen.to_string(),
            lines: 1,
            infinite: None,
            root_fen: None,
            moves: Some(Vec::new()),
            client_id: None,
            variant: None,
        };
        let mut engine = PositionEngine {
            context: super::context_of(&request),
            wanted: Some(request),
            client_id: 7,
            ..PositionEngine::default()
        };
        let context = engine.context.clone();
        let update = |client: u64, id: u64, fen: &str| {
            serde_json::from_value(json!({
                "clientId": client, "id": id, "fen": fen, "context": context,
                "depth": 10.0, "lines": [{ "rank": 1, "depth": 10.0, "pv": ["e7e5"], "cp": 20.0 }],
                "done": false, "engine": "stockfish",
            }))
            .expect("an update")
        };
        let (stale, other, mine) = (
            update(6, 1, fen),
            update(7, 1, "8/8/8/8/8/8/8/8 w"),
            update(7, 2, fen),
        );
        assert!(!engine.receive(&stale));
        assert!(!engine.receive(&other));
        assert!(engine.receive(&mine));
        assert!(engine.busy);
        assert_eq!(engine.best().unwrap().cp, Some(20.));
        // An older update of the same search arriving late is dropped.
        assert!(!engine.receive(&update(7, 1, fen)));
    }
}
