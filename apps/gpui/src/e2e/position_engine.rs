//! The position engine (`kchess_board::position_engine`) over the app's scripted engine.

use kchess_contracts::types::AnalysisRequest;

use super::Harness;
use crate::position_engine::PositionEngine;

fn search(fen: &str) -> AnalysisRequest {
    AnalysisRequest {
        fen: fen.to_string(),
        lines: 1,
        infinite: Some(true),
        root_fen: None,
        moves: Some(Vec::new()),
        client_id: None,
        variant: None,
    }
}

/// The core runs calls in any order, and a stop that landed while the next search was starting
/// aborted it: on CI, Guess the move judged a guess with no engine lines (2026-11-17). A stop
/// names the search it ends, so the stop for the earlier search, run after the next one started,
/// leaves that one searching.
#[gpui_kit::test]
fn a_stop_that_lands_after_the_next_search_leaves_it_searching(cx: &mut gpui_kit::TestAppContext) {
    use crate::core::{AppCore, ClientEvent, CoreEvent};
    use gpui_kit::AppContext as _;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Owner {
        engine: PositionEngine,
        /// The client ids of the searches the core has ended.
        ended: Vec<Option<i64>>,
        _events: gpui_kit::Subscription,
    }
    fn engine_of(owner: &mut Owner) -> &mut PositionEngine {
        &mut owner.engine
    }
    let mut app = Harness::start(cx);
    // As every page searching does: the core refuses analysis until its startup check for a
    // live Lichess game is over, and a search sent sooner came back refused (1 run in 10).
    app.wait_for("the startup check to finish", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
    let owner = app.cx.update(|cx| {
        let events = cx.global::<AppCore>().events.clone();
        cx.new(|cx| Owner {
            engine: PositionEngine::default(),
            ended: Vec::new(),
            _events: cx.subscribe(&events, |owner: &mut Owner, _, event: &ClientEvent, cx| {
                let ClientEvent::Event(CoreEvent::Analysis(update)) = event else {
                    return;
                };
                if update.done {
                    owner.ended.push(update.client_id);
                }
                if owner.engine.receive(update) {
                    cx.notify();
                }
            }),
        })
    });
    // The scripted engine answers a search at once.
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    app.cx.update(|cx| {
        owner.update(cx, |owner, cx| {
            owner.engine.search(Some(search(start)), engine_of, cx)
        })
    });
    // A refused search fails here with its reason, not after the wait's timeout.
    app.wait_for("the first search's lines or its error", |_, cx| {
        let engine = &owner.read(cx).engine;
        engine.best().is_some() || engine.error.is_some()
    });
    let error = app.cx.update(|cx| owner.read(cx).engine.error.clone());
    assert_eq!(error, None, "the first search was refused");

    let client = |app: &mut Harness| {
        app.cx.update(|cx| {
            owner
                .read(cx)
                .engine
                .update
                .as_ref()
                .and_then(|update| u64::try_from(update.client_id?).ok())
        })
    };
    let first = client(&mut app);
    assert!(
        first.is_some(),
        "the first search's updates name its client"
    );

    // The view stops, then asks for the next position, as Guess the move does between guesses.
    let after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    app.cx.update(|cx| {
        owner.update(cx, |owner, cx| {
            owner.engine.search(None, engine_of, cx);
            owner.engine.search(Some(search(after_e4)), engine_of, cx);
        })
    });
    app.wait_for("the next search's lines", |_, cx| {
        let engine = &owner.read(cx).engine;
        engine.best().is_some() || engine.error.is_some()
    });
    let next = client(&mut app);
    assert_ne!(next, first);

    // The stop for the first search runs only now, after the next search started.
    let answered = Arc::new(AtomicBool::new(false));
    let stop = app.cx.update(|cx| {
        cx.global::<AppCore>()
            .request(kchess_contracts::calls::StopAnalysis {
                kill: Some(true),
                client: first.map(kchess_contracts::check::Bounded),
            })
    });
    let done = Arc::clone(&answered);
    app.cx
        .background_executor
        .spawn(async move {
            stop.await.expect("the stop is answered");
            done.store(true, Ordering::SeqCst);
        })
        .detach();
    app.wait_for("the late stop's answer", |_, _| {
        answered.load(Ordering::SeqCst)
    });
    app.settle();
    let (busy, error) = app.cx.update(|cx| {
        let engine = &owner.read(cx).engine;
        (engine.busy, engine.error.clone())
    });
    assert_eq!(error, None);
    assert!(busy, "the late stop ended the next search");
    let next = next.and_then(|next| i64::try_from(next).ok());
    assert!(
        app.cx.update(|cx| !owner.read(cx).ended.contains(&next)),
        "the core ended the next search"
    );

    // The view's own stop ends it.
    app.cx
        .update(|cx| owner.update(cx, |owner, cx| owner.engine.search(None, engine_of, cx)));
    app.wait_for("the view's stop to end its search", |_, cx| {
        owner.read(cx).ended.contains(&next)
    });
}
