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

/// A search asked for after a stop starts only once the core has answered the stop. The core
/// runs calls in any order, and a stop that landed while the next search was starting aborted
/// it: on CI, Guess the move judged a guess with no engine lines (2026-11-17).
#[gpui_kit::test]
fn a_search_after_a_stop_waits_for_its_answer(cx: &mut gpui_kit::TestAppContext) {
    use crate::core::{AppCore, ClientEvent, CoreEvent};
    use futures::FutureExt as _;
    use gpui_kit::AppContext as _;

    struct Owner {
        engine: PositionEngine,
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
            _events: cx.subscribe(&events, |owner: &mut Owner, _, event: &ClientEvent, cx| {
                if let ClientEvent::Event(CoreEvent::Analysis(update)) = event
                    && owner.engine.receive(update)
                {
                    cx.notify();
                }
            }),
        })
    });
    let found = |app: &mut Harness| app.cx.update(|cx| owner.read(cx).engine.best().is_some());
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

    // A stop the core has not answered yet.
    let (answer, answered) = futures::channel::oneshot::channel::<()>();
    let after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    app.cx.update(|cx| {
        owner.update(cx, |owner, cx| {
            owner.engine.wait_for_stop(answered.map(drop));
            owner.engine.search(Some(search(after_e4)), engine_of, cx);
        })
    });
    for _ in 0..25 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.cx.run_until_parked();
        assert!(
            !found(&mut app),
            "the search started before the stop was answered"
        );
    }
    answer.send(()).expect("the search waits for the stop");
    app.wait_for("the search's lines once the stop is answered", |_, cx| {
        owner.read(cx).engine.best().is_some()
    });
    let error = app.cx.update(|cx| owner.read(cx).engine.error.clone());
    assert_eq!(error, None);
}
