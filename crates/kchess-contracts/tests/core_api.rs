//! The core's methods as one declaration makes them: the typed calls a frontend sends, the
//! argument check every call passes before it runs, and the dispatch that runs it.

use std::sync::Mutex;

use kchess_contracts::calls::{self, CoreCall};
use kchess_contracts::core_api::{self, Assist, METHODS, check_arguments, dispatch};
use kchess_contracts::types::ProfileArea;
use serde_json::{Value, json};

fn check(method: &str, args: &[Value]) -> Option<Result<Vec<Value>, String>> {
    check_arguments(
        method,
        kchess_contracts::core_api::as_checked(args.to_vec()),
    )
}

fn passes(method: &str, args: &[Value]) -> bool {
    matches!(check(method, args), Some(Ok(_)))
}

#[test]
fn method_names_are_unique() {
    let mut names: Vec<&str> = METHODS.iter().map(|m| m.name).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total);
    let lookup = core_api::method("positionLookup").expect("declared");
    let params: Vec<(&str, bool)> = lookup.params.iter().map(|p| (p.name, p.optional)).collect();
    assert_eq!(params, [("kind", false), ("fen", false), ("options", true)]);
    assert_eq!(lookup.assist, Some(Assist::Lookups));
    assert_eq!(
        core_api::method("saveSession").expect("declared").writes,
        [ProfileArea::Sessions]
    );
}

#[test]
fn excess_and_missing_arguments_are_refused_before_any_check() {
    assert_eq!(
        check("answerMistake", &[json!(1), json!(2), json!(3)]),
        Some(Err("Invalid argument count for answerMistake.".into()))
    );
    assert_eq!(
        check("answerMistake", &[]),
        Some(Err("Invalid argument count for answerMistake.".into()))
    );
    assert_eq!(check("unknownMethod", &[]), None);
}

/// An optional argument left out is `null` to the method; one given is checked.
#[test]
fn an_optional_argument_is_checked_only_when_given() {
    assert_eq!(check("stopAnalysis", &[]), Some(Ok(vec![Value::Null])));
    assert_eq!(
        check("stopAnalysis", &[Value::Null]),
        Some(Ok(vec![Value::Null]))
    );
    assert_eq!(
        check("stopAnalysis", &[json!(true)]),
        Some(Ok(vec![json!(true)]))
    );
    assert!(!passes("stopAnalysis", &[json!("yes")]));
    // A username is trimmed by its check.
    assert_eq!(
        check("syncGames", &[json!(" alice ")]),
        Some(Ok(vec![json!("alice")]))
    );
}

/// A variant's position is looked up by its own rules: a Crazyhouse FEN with its pockets passes
/// with its variant and is refused without one; an unknown variant is refused.
#[test]
fn a_position_lookup_checks_a_variant_position_by_its_rules() {
    let zh = json!("rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3");
    let lookup = |fen: &Value, options: Value| {
        passes("positionLookup", &[json!("opening"), fen.clone(), options])
    };
    assert!(lookup(&zh, json!({ "variant": "crazyhouse" })));
    assert!(!lookup(&zh, json!({})));
    assert!(!lookup(&zh, json!({ "variant": "bughouse" })));
    let start = json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    assert!(lookup(&start, json!({ "variant": "standard" })));
    // The arguments reach the lookup as sent.
    assert_eq!(
        check("positionLookup", &[json!("opening"), start.clone()]),
        Some(Ok(vec![json!("opening"), start, Value::Null]))
    );
}

/// A stored review of a variant game is checked with its moves by the variant's rules.
#[test]
fn a_stored_review_checks_a_variant_game_by_its_rules() {
    let zh = json!("rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3");
    assert!(passes(
        "reviewGet",
        &[zh.clone(), json!([]), json!("crazyhouse")]
    ));
    assert!(!passes("reviewGet", &[zh, json!([])]));
    let start = json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    assert!(passes("reviewGet", &[start.clone(), json!(["e2e4"])]));
    assert!(!passes("reviewGet", &[start, json!(["e9e4"])]));
}

#[test]
fn speech_recognition_takes_a_bounded_grammar_a_sample_rate_and_base64_audio() {
    assert!(passes(
        "voiceStart",
        &[json!(["e four", "[unk]"]), json!(16_000)]
    ));
    for (grammar, rate) in [
        (json!([]), json!(16_000)),
        (json!(["e four", 4]), json!(16_000)),
        (json!(["a".repeat(201)]), json!(16_000)),
        (json!(vec!["e four"; 4097]), json!(16_000)),
        (json!(["e four"]), json!(7_999)),
        (json!(["e four"]), json!("16000")),
    ] {
        assert!(
            !passes("voiceStart", &[grammar.clone(), rate.clone()]),
            "{grammar} {rate}"
        );
    }
    assert!(passes("voiceAccept", &[json!("AAD/fw==")]));
    // The largest chunk decodes to just under the helper's 1 MiB frame; one more quad is over.
    let largest = (1 << 20) / 3 * 4;
    assert!(passes("voiceAccept", &[json!("A".repeat(largest))]));
    for audio in [
        json!("AAD"),
        json!("AA-_"),
        json!(12),
        json!("A".repeat(largest + 4)),
    ] {
        assert_eq!(
            check("voiceAccept", &[audio]),
            Some(Err("Invalid audio.".into()))
        );
    }
    for method in [
        "voiceFinish",
        "voiceStop",
        "voiceModelStatus",
        "voiceModelInstall",
    ] {
        assert!(passes(method, &[]), "{method}");
    }
}

#[test]
fn a_call_sends_its_arguments_in_order_without_trailing_ones_left_out() {
    assert!(calls::DesktopSettings {}.args().expect("args").is_empty());
    assert!(
        calls::StopAnalysis { kill: None }
            .args()
            .expect("args")
            .is_empty()
    );
    assert_eq!(
        calls::StopAnalysis { kill: Some(true) }
            .args()
            .expect("args"),
        [json!(true)]
    );
    // One left out between two given is sent as `null`, which the check reads alike.
    let command = calls::StudyCommand {
        command: kchess_contracts::types::StudyCommand::Duplicate { id: "a".into() },
        incremental: None,
        based_on: Some(kchess_contracts::check::PgnText("abc".into())),
    };
    assert_eq!(
        command.args().expect("args"),
        [
            json!({ "op": "duplicate", "id": "a" }),
            Value::Null,
            json!("abc")
        ]
    );
    assert_eq!(calls::ReviewGet::METHOD, "reviewGet");
}

/// An argument that cannot be written as JSON fails the call with its cause; it was sent as
/// `null`, and the core refused it with a check's message that named another problem.
#[test]
fn an_argument_that_cannot_be_written_is_an_error_naming_its_method() {
    let keyed_by_pairs: std::collections::HashMap<(u8, u8), u8> = [((1, 2), 3)].into();
    let refused =
        calls::arg("saveSettings", keyed_by_pairs).expect_err("a map keyed by pairs is not JSON");
    assert!(
        refused
            .message
            .starts_with("An argument of saveSettings could not be written"),
        "{}",
        refused.message
    );
}

#[test]
fn an_answer_is_read_in_the_call_s_shape() {
    let report = json!({ "themes": [], "dir": "/themes", "problems": ["bad.json: no name"] });
    let read = calls::decode::<calls::CustomThemes>(report).expect("a report");
    assert_eq!(read.dir, "/themes");
    let failed = calls::decode::<calls::SaveDesktopSettings>(json!(12)).expect_err("not settings");
    assert!(
        failed
            .message
            .starts_with("The core's answer to saveDesktopSettings could not be read: "),
        "{}",
        failed.message
    );
}

/// A core that records what the dispatch asked of it.
#[derive(Default)]
struct Recorder {
    blocked: Mutex<Vec<bool>>,
    refused: Mutex<Vec<String>>,
    changed: Mutex<Vec<&'static [ProfileArea]>>,
}

impl Recorder {
    fn blocked_after(&self, checks: &[bool]) {
        *self.blocked.lock().unwrap() = checks.iter().rev().copied().collect();
    }
}

#[test]
fn the_dispatch_refuses_unknown_methods_and_bad_arguments() {
    let api = recorder::Api::default();
    let run = |method: &str, args: Vec<Value>| futures_lite_block_on(dispatch(&api, method, args));
    assert_eq!(
        run("noSuchMethod", vec![]).unwrap_err().message,
        "Unknown core method noSuchMethod."
    );
    assert!(run("answerMistake", vec![json!(1)]).is_err());
    assert_eq!(
        api.0.refused.lock().unwrap().as_slice(),
        ["answerMistake: Invalid argument count for answerMistake."]
    );
}

#[test]
fn the_dispatch_gates_assistance_before_and_after_and_reports_writes() {
    let api = recorder::Api::default();
    let fen = json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    // Blocked before it runs: the stored review is refused until the game status is known.
    api.0.blocked_after(&[true]);
    let refused = futures_lite_block_on(dispatch(&api, "reviewGet", vec![fen.clone(), json!([])]));
    assert_eq!(
        refused.unwrap_err().message,
        "Review is unavailable until your Lichess game status is verified."
    );
    assert_eq!(api.calls(), Vec::<String>::new());
    // A game started while it ran: the answer is held back.
    api.0.blocked_after(&[false, true]);
    let held = futures_lite_block_on(dispatch(&api, "reviewGet", vec![fen.clone(), json!([])]));
    assert_eq!(
        held.unwrap_err().message,
        "Review is unavailable during a live Lichess game."
    );
    assert_eq!(api.calls(), ["reviewGet"]);
    api.0.blocked_after(&[false, false]);
    let answered = futures_lite_block_on(dispatch(&api, "reviewGet", vec![fen, json!([])]));
    assert_eq!(answered, Ok(Value::Null));
    assert_eq!(api.calls(), ["reviewGet"]);
    // A write reports what it changed, and only once it succeeded.
    let saved = futures_lite_block_on(dispatch(&api, "removeArchivedGame", vec![json!("game-1")]));
    assert_eq!(saved, Ok(Value::Null));
    assert_eq!(api.calls(), ["removeArchivedGame"]);
    assert_eq!(
        api.0.changed.lock().unwrap().as_slice(),
        [&[ProfileArea::PlayedGames][..]]
    );
}

/// A typed argument is refused with its type's message, reported as a refused call.
#[test]
fn a_typed_argument_is_refused_with_its_message() {
    let api = recorder::Api::default();
    let refused = futures_lite_block_on(dispatch(&api, "removeArchivedGame", vec![json!("")]));
    assert_eq!(
        refused.unwrap_err().message,
        "invalid value, expected at least 1 character"
    );
    let kind = futures_lite_block_on(dispatch(
        &api,
        "positionLookup",
        vec![json!("bogus"), json!("8/8/8/8/8/8/8/K6k w - - 0 1")],
    ));
    assert_eq!(
        kind.unwrap_err().message,
        "invalid value, expected one of opening, masters, player, tablebase"
    );
    assert_eq!(api.calls(), Vec::<String>::new());
    assert_eq!(api.0.refused.lock().unwrap().len(), 2);
}

/// A call made as a Lichess account answers its result, or that the account must connect again.
#[test]
fn a_reply_crosses_as_its_result_or_needs_reconnect() {
    use kchess_contracts::types::{LichessStudy, Reply};
    let done = Reply::Done(vec![LichessStudy {
        id: "abcdEFGH".into(),
        name: "Endgames".into(),
        updated_at: 9,
    }]);
    let wire = r#"[{"id":"abcdEFGH","name":"Endgames","updatedAt":9}]"#;
    assert_eq!(serde_json::to_string(&done).unwrap(), wire);
    assert_eq!(
        serde_json::from_str::<Reply<Vec<LichessStudy>>>(wire).unwrap(),
        done
    );
    let reconnect = Reply::<Vec<LichessStudy>>::NeedsReconnect;
    let wire = r#"{"needsReconnect":true}"#;
    assert_eq!(serde_json::to_string(&reconnect).unwrap(), wire);
    assert_eq!(
        serde_json::from_str::<Reply<Vec<LichessStudy>>>(wire).unwrap(),
        reconnect
    );
}

/// Runs a future that never waits on anything outside itself.
fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("the test core never waits"),
    }
}

#[allow(unused_variables)]
mod recorder {
    use super::*;
    use kchess_contracts::CoreError;
    use kchess_contracts::core_api::*;

    #[derive(Default)]
    pub struct Api(pub Recorder, pub Mutex<Vec<String>>);

    impl Api {
        pub fn calls(&self) -> Vec<String> {
            std::mem::take(&mut self.1.lock().unwrap())
        }

        /// Every method answers `null`, which reads as any answer that may be nothing.
        fn ran(&self, method: &str) -> Result<Value, CoreError> {
            self.1.lock().unwrap().push(method.into());
            Ok(Value::Null)
        }
    }

    impl CoreApi for Api {
        fn assistance_blocked(&self) -> bool {
            self.0.blocked.lock().unwrap().pop().unwrap_or(false)
        }
        fn refused_arguments(&self, method: &str, cause: &str) {
            self.0
                .refused
                .lock()
                .unwrap()
                .push(format!("{method}: {cause}"));
        }
        fn changed(&self, areas: &'static [ProfileArea]) {
            self.0.changed.lock().unwrap().push(areas);
        }
    }

    impl AnswerEveryMethod for Api {
        fn answer_method(&self, method: &'static str) -> Result<Value, CoreError> {
            self.ran(method)
        }
    }
}
