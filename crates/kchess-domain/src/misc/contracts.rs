//! The argument contracts of the core's methods (`CORE_CONTRACTS`): how many arguments a method
//! takes, and one validator per argument, run before the method does anything.
//!
//! The table is the single source: the facade checks every call against it
//! (`validate_arguments`), and `validateCoreArguments` answers the same check by name. A JSON
//! `null` stands for `undefined` where the facade passes JSON arguments, since JSON cannot carry
//! `undefined`.

use serde_json::{Value, json};

use super::value::J;
use super::{Run, call};

/// One argument's validator.
#[derive(Clone, Copy)]
enum Check {
    /// A validator by its name (`assertX`).
    Valid(&'static str),
    /// `optional(validator)`: an absent (`undefined`) argument passes.
    Optional(&'static str),
    /// `assertWatchTarget(value, TV_CHANNEL_KEYS)`.
    WatchTarget,
    /// `assertAnalysisRequest({ fen: value, lines: 1 })`: a position alone.
    Position,
    /// Any value: the method normalizes it itself.
    Any,
}

use Check::{Any, Optional, Position, Valid, WatchTarget};

/// A method's row: its name, the fewest arguments it takes, and one check per argument.
type Row = (&'static str, usize, &'static [Check]);

/// Which table a method is checked against.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `CORE_CONTRACTS`: the core's `CoreApi` methods.
    Core,
}

/// Methods refused during a live human game (engine, review and explorer assistance),
/// gated by the facade's `assistance_allowed` (`crates/kchess-core/src/facade.rs`). The single
/// source for the gate and the CLI's pre-call `resumeOnline`: when a facade arm gains the gate,
/// add its method here and regenerate the arities.
pub const ASSISTANCE_METHODS: &[&str] = &[
    "bestMove",
    "startAnalysis",
    "reviewGet",
    "reviewRequest",
    "positionLookup",
    "mastersGame",
    "openingDescription",
    "openingPopularity",
    "cloudEval",
    "aiExplain",
];
/// The channels `assertWatchTarget` accepts (`TV_CHANNEL_KEYS`).
pub const TV_CHANNEL_KEYS: &[&str] = &super::constants::TV_CHANNEL_KEYS;

const CORE: &[Row] = &[
    (
        "positionLookup",
        2,
        &[
            Valid("assertLookupKind"),
            Position,
            Valid("assertLookupOptions"),
        ],
    ),
    ("syncLichessStudy", 1, &[Valid("assertStudySyncRequest")]),
    (
        "lichessStudyChapter",
        2,
        &[Valid("assertLichessId"), Valid("assertStudyId")],
    ),
    ("lichessStudies", 1, &[Valid("assertUsername")]),
    (
        "lichessStudyChapters",
        2,
        &[Valid("assertUsername"), Valid("assertLichessId")],
    ),
    (
        "exportToLichessStudy",
        4,
        &[
            Valid("assertUsername"),
            Valid("assertStudyId"),
            Valid("assertChapterName"),
            Valid("assertPgnText"),
        ],
    ),
    ("exportGame", 1, &[Valid("assertGameId")]),
    ("mastersGame", 1, &[Valid("assertGameId")]),
    ("openingDescription", 1, &[Valid("assertMoves")]),
    ("openingPopularity", 1, &[Valid("assertMoves")]),
    ("cloudEval", 2, &[Position, Valid("assertLines")]),
    ("loadData", 0, &[]),
    ("saveSettings", 1, &[Valid("assertCoreSettings")]),
    ("addAccount", 1, &[Valid("assertUsername")]),
    ("logout", 1, &[Valid("assertUsername")]),
    ("logoutAll", 0, &[]),
    ("removeAccount", 1, &[Valid("assertUsername")]),
    ("syncGames", 0, &[Optional("assertUsername")]),
    ("gamePage", 1, &[Valid("assertGamePageQuery")]),
    ("gameLibraryOverview", 0, &[]),
    ("insights", 1, &[Valid("assertInsightsQuery")]),
    ("gameRatingHistory", 1, &[Valid("assertGameAccount")]),
    (
        "gamePgn",
        2,
        &[Valid("assertGameAccount"), Valid("assertGameId")],
    ),
    ("chesscomAccounts", 0, &[]),
    ("addChesscomAccount", 1, &[Valid("assertChesscomUsername")]),
    (
        "removeChesscomAccount",
        1,
        &[Valid("assertChesscomUsername")],
    ),
    (
        "syncChesscomGames",
        0,
        &[Optional("assertChesscomUsername")],
    ),
    ("cachedProfile", 1, &[Valid("assertUsername")]),
    ("profile", 1, &[Valid("assertUsername")]),
    ("ratingHistory", 1, &[Valid("assertUsername")]),
    ("connectLichess", 0, &[Any]),
    ("engineStatus", 0, &[Optional("assertVariant")]),
    ("installEngine", 0, &[]),
    ("deleteEngine", 0, &[]),
    ("stopEngine", 0, &[]),
    (
        "bestMove",
        2,
        &[
            Valid("assertMoves"),
            Valid("assertLevel"),
            Valid("assertBestMoveOptions"),
        ],
    ),
    ("startAnalysis", 1, &[Valid("assertAnalysisRequest")]),
    ("stopAnalysis", 0, &[Optional("assertBoolean")]),
    ("reviewGet", 2, &[Position, Valid("assertMoves"), Any]),
    ("reviewRequest", 1, &[Valid("assertReviewRequest")]),
    ("reviewCancel", 1, &[Valid("assertReviewKey")]),
    ("reviewStatus", 0, &[]),
    ("aiStatus", 0, &[]),
    ("aiConnect", 0, &[]),
    ("aiDisconnect", 0, &[]),
    ("aiModels", 0, &[]),
    (
        "aiSetOptions",
        2,
        &[Valid("assertAiModel"), Valid("assertAiEffort")],
    ),
    ("aiExplain", 1, &[Valid("assertAiRequest")]),
    ("reviewSummaries", 1, &[Valid("assertGameIds")]),
    ("startOnline", 1, &[Valid("assertOnlineOptions")]),
    ("resumeOnline", 0, &[]),
    ("cancelOnline", 0, &[]),
    (
        "playOnline",
        2,
        &[Valid("assertGameId"), Valid("assertUci")],
    ),
    (
        "playCorrespondence",
        4,
        &[
            Valid("assertUsername"),
            Valid("assertGameId"),
            Valid("assertFen"),
            Valid("assertUci"),
        ],
    ),
    (
        "onlineAction",
        2,
        &[Valid("assertGameId"), Valid("assertAction")],
    ),
    ("onlineChat", 1, &[Valid("assertGameId")]),
    (
        "sendChat",
        3,
        &[
            Valid("assertGameId"),
            Valid("assertChatRoom"),
            Valid("assertChatText"),
        ],
    ),
    ("stayConnected", 1, &[Valid("assertOptionalAccount")]),
    ("challenges", 0, &[]),
    ("ongoingGames", 0, &[]),
    ("acceptChallenge", 1, &[Valid("assertGameId")]),
    ("cancelChallenge", 1, &[Valid("assertGameId")]),
    (
        "declineChallenge",
        2,
        &[Valid("assertGameId"), Valid("assertDeclineReason")],
    ),
    (
        "openGame",
        2,
        &[Valid("assertUsername"), Valid("assertGameId")],
    ),
    (
        "playerPerf",
        2,
        &[Valid("assertUsername"), Valid("assertPerfType")],
    ),
    (
        "crosstable",
        2,
        &[Valid("assertUsername"), Valid("assertUsername")],
    ),
    (
        "recentGames",
        1,
        &[Valid("assertUsername"), Optional("assertBoolean")],
    ),
    (
        "sendMessage",
        3,
        &[
            Valid("assertUsername"),
            Valid("assertUsername"),
            Valid("assertMessageText"),
        ],
    ),
    (
        "relation",
        2,
        &[Valid("assertUsername"), Valid("assertUsername")],
    ),
    (
        "setRelation",
        3,
        &[
            Valid("assertUsername"),
            Valid("assertUsername"),
            Valid("assertRelationAction"),
        ],
    ),
    ("profiles", 1, &[Valid("assertFriendList")]),
    ("leaderboard", 1, &[Valid("assertPerfType")]),
    ("searchPlayers", 1, &[Valid("assertUsername")]),
    ("activity", 1, &[Valid("assertUsername")]),
    ("timeline", 0, &[]),
    (
        "importGame",
        2,
        &[Valid("assertOptionalAccount"), Valid("assertPgnText")],
    ),
    ("bookmarkedGames", 1, &[Valid("assertUsername")]),
    ("tvChannels", 0, &[]),
    ("stopWatching", 0, &[Optional("assertWatchSession")]),
    ("watch", 1, &[WatchTarget]),
    ("watchBroadcast", 1, &[Valid("assertLichessId")]),
    ("broadcasts", 0, &[Valid("assertBroadcastQuery")]),
    ("broadcastTour", 1, &[Valid("assertLichessId")]),
    ("broadcastRound", 1, &[Valid("assertLichessId")]),
    ("tournaments", 1, &[Valid("assertOptionalAccount")]),
    (
        "tournament",
        3,
        &[
            Valid("assertTournamentSystem"),
            Valid("assertTournamentId"),
            Valid("assertOptionalAccount"),
            Valid("assertOptionalPage"),
        ],
    ),
    (
        "joinTournament",
        3,
        &[
            Valid("assertTournamentSystem"),
            Valid("assertTournamentId"),
            Valid("assertUsername"),
            Valid("assertTournamentPassword"),
        ],
    ),
    (
        "leaveTournament",
        3,
        &[
            Valid("assertTournamentSystem"),
            Valid("assertTournamentId"),
            Valid("assertUsername"),
        ],
    ),
    (
        "createTournament",
        2,
        &[Valid("assertUsername"), Valid("assertNewArena")],
    ),
    ("clearAccountData", 1, &[Valid("assertUsername")]),
    ("following", 0, &[]),
    ("addFriends", 1, &[Valid("assertFriendList")]),
    ("usage", 0, &[]),
    ("resetUsage", 0, &[]),
    ("presence", 1, &[Valid("assertUsernames")]),
    ("puzzleNext", 1, &[Valid("assertPuzzleRequest")]),
    ("puzzleSolve", 1, &[Valid("assertPuzzleSolve")]),
    ("puzzleDaily", 0, &[]),
    ("puzzleById", 1, &[Valid("assertPuzzleId")]),
    (
        "puzzleDashboard",
        2,
        &[Valid("assertUsername"), Valid("assertDays")],
    ),
    (
        "puzzleActivity",
        2,
        &[Valid("assertUsername"), Valid("assertActivityMax")],
    ),
    (
        "stormDashboard",
        2,
        &[Valid("assertUsername"), Valid("assertDays")],
    ),
    ("puzzleDbStatus", 0, &[]),
    ("puzzleDbInstall", 0, &[]),
    ("puzzleDbCancel", 0, &[]),
    ("puzzleDbDelete", 0, &[]),
    ("localPuzzles", 1, &[Valid("assertLocalQuery")]),
    ("localLadder", 1, &[Valid("assertLadderQuery")]),
    ("recordPuzzleAttempt", 1, &[Valid("assertPuzzleAttempt")]),
    ("puzzleProgress", 1, &[Valid("assertPuzzleProgressQuery")]),
    ("saveRun", 1, &[Valid("assertRunInput")]),
    ("runSummary", 1, &[Valid("assertRunKind")]),
    ("clearRuns", 0, &[Optional("assertRunKind")]),
    ("saveVoiceAttempt", 1, &[Valid("assertVoiceAttempt")]),
    (
        "updateVoiceAttempt",
        2,
        &[Valid("assertVoiceId"), Valid("assertVoiceUpdate")],
    ),
    ("voiceHistory", 1, &[Valid("assertVoiceLimit")]),
    ("clearVoiceHistory", 0, &[]),
    (
        "voiceStart",
        2,
        &[Valid("assertVoiceGrammar"), Valid("assertSampleRate")],
    ),
    ("voiceAccept", 1, &[Valid("assertVoiceAudio")]),
    ("voiceFinish", 0, &[]),
    ("voiceStop", 0, &[]),
    ("voiceModelStatus", 0, &[]),
    ("voiceModelInstall", 0, &[]),
    ("library", 0, &[]),
    (
        "studyCommand",
        1,
        &[
            Valid("assertStudyCommandShape"),
            Optional("assertBoolean"),
            Optional("assertPgnText"),
        ],
    ),
    ("saveArchivedGame", 1, &[Valid("assertArchivedGameShape")]),
    ("removeArchivedGame", 1, &[Valid("assertLibraryId")]),
    (
        "addMistakes",
        1,
        &[Valid("assertReviewKey"), Optional("assertSide")],
    ),
    (
        "answerMistake",
        2,
        &[Valid("assertLibraryId"), Valid("assertBoolean")],
    ),
    ("mistakeGames", 0, &[]),
    (
        "saveSession",
        2,
        &[Valid("assertSessionKind"), Valid("assertSessionObject")],
    ),
    ("joinedTournaments", 0, &[]),
    (
        "recordRepertoireMiss",
        2,
        &[Valid("assertRepertoireKey"), Position],
    ),
    ("clearRepertoireMisses", 1, &[Valid("assertRepertoireKey")]),
    (
        "restoreRepertoireMisses",
        2,
        &[Valid("assertRepertoireKey"), Valid("assertSessionObject")],
    ),
    (
        "reviewRepertoireLine",
        3,
        &[
            Valid("assertRepertoireKey"),
            Valid("assertRepertoireLine"),
            Valid("assertBoolean"),
        ],
    ),
    ("viewPreferences", 0, &[]),
    ("customThemes", 0, &[]),
    (
        "setViewPreference",
        2,
        &[Valid("assertViewPreferenceKey"), Any],
    ),
];

/// The row of `method` in `scope`, or `None` when it has no contract there.
fn row(scope: Scope, method: &str) -> Option<(usize, &'static [Check])> {
    let find = |rows: &'static [Row]| {
        rows.iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, min, checks)| (*min, *checks))
    };
    match scope {
        Scope::Core => find(CORE),
    }
}

/// Every method of `scope` with its fewest and most arguments, sorted by name.
pub fn arities(scope: Scope) -> Vec<(&'static str, usize, usize)> {
    let rows: &[Row] = match scope {
        Scope::Core => CORE,
    };
    let mut out: Vec<(&'static str, usize, usize)> = rows
        .iter()
        .map(|(name, min, checks)| (*name, *min, checks.len()))
        .collect();
    out.sort_by_key(|(name, ..)| *name);
    out
}

/// Runs validator `name` on `args`; a rejection is its message.
fn apply(name: &str, args: Vec<J>) -> Result<Value, String> {
    match Run::find(name) {
        Some(validator) => validator.call(args),
        None => Err(format!("Unknown validator {name}.")),
    }
}

/// Runs one argument's check on `value` (`undefined` when the argument is absent).
fn run_check(check: Check, value: &J) -> Result<(), String> {
    match check {
        Valid(name) => apply(name, vec![value.clone()]).map(|_| ()),
        Optional(_) if matches!(value, J::Undef) => Ok(()),
        Optional(name) => apply(name, vec![value.clone()]).map(|_| ()),
        WatchTarget => apply(
            "assertWatchTarget",
            vec![
                value.clone(),
                J::Arr(
                    TV_CHANNEL_KEYS
                        .iter()
                        .map(|key| J::Str((*key).into()))
                        .collect(),
                ),
            ],
        )
        .map(|_| ()),
        Position => apply(
            "assertAnalysisRequest",
            vec![J::Obj(vec![
                ("fen".into(), value.clone()),
                ("lines".into(), J::Num(1.0)),
            ])],
        )
        .map(|_| ()),
        Any => Ok(()),
    }
}

/// Checks a method's arguments: the count first, then each argument in order, and the first
/// failing check's message is the result (as `validateArguments` throws it).
pub fn check(scope: Scope, method: &str, args: &[J]) -> Result<(), String> {
    let count_error = || Err(format!("Invalid argument count for {method}."));
    let Some((min, checks)) = row(scope, method) else {
        return count_error();
    };
    if args.len() < min || args.len() > checks.len() {
        return count_error();
    }
    // `reviewGet(fen, moves, variant)`: a variant game's position is checked by its own rules.
    if method == "reviewGet"
        && let Some(J::Str(variant)) = args.get(2)
    {
        return apply(
            "assertReviewRequest",
            vec![J::Obj(vec![
                ("fen".into(), args[0].clone()),
                ("moves".into(), args[1].clone()),
                ("variant".into(), J::Str(variant.clone())),
            ])],
        )
        .map(|_| ());
    }
    // `positionLookup(kind, fen, { variant })`: a variant's position is checked by its own rules.
    if method == "positionLookup"
        && let Some(J::Obj(options)) = args.get(2)
        && let Some((_, J::Str(variant))) = options.iter().find(|(key, _)| key == "variant")
        && variant != "standard"
    {
        run_check(Valid("assertLookupKind"), &args[0])?;
        run_check(Valid("assertLookupOptions"), &args[2])?;
        return apply(
            "assertAnalysisRequest",
            vec![J::Obj(vec![
                ("fen".into(), args[1].clone()),
                ("lines".into(), J::Num(1.0)),
                ("variant".into(), J::Str(variant.clone())),
            ])],
        )
        .map(|_| ());
    }
    let absent = J::Undef;
    for (index, check) in checks.iter().enumerate() {
        run_check(*check, args.get(index).unwrap_or(&absent))?;
    }
    if method == "reviewGet" {
        // `assertReviewRequest({ fen: args[0], moves: args[1] })`: the pair must be a review request.
        apply(
            "assertReviewRequest",
            vec![J::Obj(vec![
                ("fen".into(), args[0].clone()),
                ("moves".into(), args[1].clone()),
            ])],
        )?;
    }
    Ok(())
}

/// Checks a core method's JSON arguments, as the facade receives them (a JSON `null` is
/// `undefined`). `None` when the method has no core contract, so the facade dispatches it.
pub fn validate_arguments(method: &str, args: &[Value]) -> Option<Result<(), String>> {
    row(Scope::Core, method)?;
    let values: Vec<J> = args
        .iter()
        .map(|value| {
            if value.is_null() {
                J::Undef
            } else {
                super::from_json(value)
            }
        })
        .collect();
    Some(check(Scope::Core, method, &values))
}

/// Whether `method` is the argument-check call (`validateCoreArguments`), which takes
/// `[method, args]`.
pub fn is_call(method: &str) -> bool {
    method == "validateCoreArguments"
}

/// The argument-check call. `values` are `[method, args]`, with `undefined` kept where the
/// caller sent it. A valid call returns `null`.
pub fn argument_call(values: &[J]) -> Result<J, String> {
    match values {
        [J::Str(name), J::Arr(list)] => check(Scope::Core, name, list).map(|()| J::Null),
        _ => Err("The arguments of an argument check are a method name and a list.".into()),
    }
}

/// The normalized value validator `name` gives for `args` (`assertX(...)` in the TypeScript).
pub fn normalize(name: &str, args: &[Value]) -> Result<Value, String> {
    run(name, args)
}

/// Runs validator `name` on `args`; a JSON `null` is `undefined`, as an absent argument is.
fn run(name: &str, args: &[Value]) -> Result<Value, String> {
    let sidecar = Value::Array(
        args.iter()
            .enumerate()
            .filter(|(_, value)| value.is_null())
            .map(|(index, _)| json!({ "at": [index], "k": 0 }))
            .collect(),
    );
    let mut lossless = vec![sidecar];
    lossless.extend(args.iter().cloned());
    match call(name, &lossless) {
        Some(result) => result,
        None => Err(format!("Unknown validator {name}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_names_a_validator() {
        for (method, _, checks) in CORE {
            for check in checks.iter() {
                if let Valid(name) | Optional(name) = check {
                    assert!(
                        Run::find(name).is_some(),
                        "{method} names no validator {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn excess_and_missing_arguments_are_rejected_before_any_check() {
        let excess = vec![J::Undef; 3];
        assert_eq!(
            check(Scope::Core, "answerMistake", &excess),
            Err("Invalid argument count for answerMistake.".into())
        );
        assert_eq!(
            check(Scope::Core, "unknownMethod", &[]),
            Err("Invalid argument count for unknownMethod.".into())
        );
    }

    /// A variant's position is looked up by its own rules: a Crazyhouse FEN with its pockets
    /// passes with its variant and is refused without one; an unknown variant is refused.
    #[test]
    fn a_position_lookup_checks_a_variant_position_by_its_rules() {
        let zh = json!("rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3");
        let lookup = |fen: &Value, options: Value| {
            validate_arguments("positionLookup", &[json!("opening"), fen.clone(), options])
        };
        assert_eq!(
            lookup(&zh, json!({ "variant": "crazyhouse" })),
            Some(Ok(()))
        );
        assert!(matches!(lookup(&zh, json!({})), Some(Err(_))));
        assert!(matches!(
            lookup(&zh, json!({ "variant": "bughouse" })),
            Some(Err(_))
        ));
        let start = json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        assert_eq!(
            lookup(&start, json!({ "variant": "standard" })),
            Some(Ok(()))
        );
    }

    #[test]
    fn speech_recognition_takes_a_bounded_grammar_a_sample_rate_and_base64_audio() {
        let ok = |method: &str, args: &[Value]| validate_arguments(method, args);
        assert_eq!(
            ok("voiceStart", &[json!(["e four", "[unk]"]), json!(16_000)]),
            Some(Ok(()))
        );
        for (grammar, rate) in [
            (json!([]), json!(16_000)),
            (json!(["e four", 4]), json!(16_000)),
            (json!(["a".repeat(201)]), json!(16_000)),
            (json!(vec!["e four"; 4097]), json!(16_000)),
            (json!(["e four"]), json!(7_999)),
            (json!(["e four"]), json!("16000")),
        ] {
            assert!(
                matches!(
                    ok("voiceStart", &[grammar.clone(), rate.clone()]),
                    Some(Err(_))
                ),
                "{grammar} {rate}"
            );
        }
        assert_eq!(ok("voiceAccept", &[json!("AAD/fw==")]), Some(Ok(())));
        // The largest chunk decodes to just under the helper's 1 MiB frame; one more quad is over.
        let largest = (1 << 20) / 3 * 4;
        assert_eq!(
            ok("voiceAccept", &[json!("A".repeat(largest))]),
            Some(Ok(()))
        );
        for audio in [
            json!("AAD"),
            json!("AA-_"),
            json!(12),
            json!("A".repeat(largest + 4)),
        ] {
            assert_eq!(
                ok("voiceAccept", &[audio]),
                Some(Err("Invalid audio.".into()))
            );
        }
        assert_eq!(ok("voiceFinish", &[]), Some(Ok(())));
        assert_eq!(ok("voiceStop", &[]), Some(Ok(())));
        assert_eq!(ok("voiceModelStatus", &[]), Some(Ok(())));
        assert_eq!(ok("voiceModelInstall", &[]), Some(Ok(())));
    }
}
