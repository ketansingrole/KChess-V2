//! The argument contracts of the core's methods (`CORE_CONTRACTS` in
//! `core/src/contracts/apiContracts.ts`): how many arguments a method takes, and one validator per
//! argument, run before the method does anything. A JSON `null` stands for `undefined` here, as an
//! absent argument does, since JSON cannot carry `undefined`.

use serde_json::{Value, json};

use super::call;

/// One argument's validator.
#[derive(Clone, Copy)]
enum Check {
    /// A validator by its TypeScript name, given the argument as it was sent.
    Valid(&'static str),
    /// `optional(validator)`: an absent argument passes.
    Optional(&'static str),
    /// `assertWatchTarget(value, TV_CHANNEL_KEYS)`.
    WatchTarget,
    /// `assertAnalysisRequest({ fen: value, lines: 1 })`: a position alone.
    Position,
    /// Any value: the method normalizes it itself.
    Any,
}

use Check::{Any, Optional, Position, Valid, WatchTarget};

/// The channels `assertWatchTarget` accepts (`TV_CHANNEL_KEYS`, `core/src/domain/tvChannels.ts`).
pub const TV_CHANNEL_KEYS: &[&str] = &[
    "best",
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "ultraBullet",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "crazyhouse",
    "bot",
    "computer",
];

/// The arguments a method takes: the fewest it accepts, then one check per argument.
struct Contract {
    min: usize,
    checks: &'static [Check],
}

/// The contract of a method, or `None` when it has none.
fn contract(method: &str) -> Option<Contract> {
    let (min, checks): (usize, &'static [Check]) = match method {
        "positionLookup" => (
            2,
            &[
                Valid("assertLookupKind"),
                Position,
                Valid("assertLookupOptions"),
            ],
        ),
        "syncLichessStudy" => (1, &[Valid("assertStudySyncRequest")]),
        "lichessStudies" => (1, &[Valid("assertUsername")]),
        "lichessStudyChapters" => (2, &[Valid("assertUsername"), Valid("assertLichessId")]),
        "exportToLichessStudy" => (
            4,
            &[
                Valid("assertUsername"),
                Valid("assertStudyId"),
                Valid("assertChapterName"),
                Valid("assertPgnText"),
            ],
        ),
        "exportGame" | "mastersGame" => (1, &[Valid("assertGameId")]),
        "cloudEval" => (2, &[Position, Valid("assertLines")]),
        "loadData" | "logoutAll" | "gameLibraryOverview" => (0, &[]),
        "saveSettings" => (1, &[Valid("assertCoreSettings")]),
        "addAccount" | "logout" | "removeAccount" => (1, &[Valid("assertUsername")]),
        "syncGames" => (0, &[Optional("assertUsername")]),
        "gamePage" => (1, &[Valid("assertGamePageQuery")]),
        "insights" => (1, &[Valid("assertInsightsQuery")]),
        "gameRatingHistory" | "cachedProfile" | "profile" | "ratingHistory" => {
            (1, &[Valid("assertUsername")])
        }
        "gamePgn" => (2, &[Valid("assertUsername"), Valid("assertGameId")]),
        "connectLichess" => (0, &[Any]),
        "engineStatus" | "installEngine" | "deleteEngine" | "stopEngine" => (0, &[]),
        "bestMove" => (
            2,
            &[
                Valid("assertMoves"),
                Valid("assertLevel"),
                Valid("assertBestMoveOptions"),
            ],
        ),
        "startAnalysis" => (1, &[Valid("assertAnalysisRequest")]),
        "stopAnalysis" => (0, &[]),
        "reviewGet" => (2, &[Position, Valid("assertMoves")]),
        "reviewRequest" => (1, &[Valid("assertReviewRequest")]),
        "reviewCancel" => (1, &[Valid("assertReviewKey")]),
        "reviewStatus" => (0, &[]),
        "reviewSummaries" => (1, &[Valid("assertGameIds")]),
        "startOnline" => (1, &[Valid("assertOnlineOptions")]),
        "resumeOnline" | "cancelOnline" => (0, &[]),
        "playOnline" => (2, &[Valid("assertGameId"), Valid("assertUci")]),
        "onlineAction" => (2, &[Valid("assertGameId"), Valid("assertAction")]),
        "onlineChat" => (1, &[Valid("assertGameId")]),
        "sendChat" => (
            3,
            &[
                Valid("assertGameId"),
                Valid("assertChatRoom"),
                Valid("assertChatText"),
            ],
        ),
        "stayConnected" => (1, &[Valid("assertOptionalAccount")]),
        "challenges" | "ongoingGames" => (0, &[]),
        "acceptChallenge" | "cancelChallenge" => (1, &[Valid("assertGameId")]),
        "declineChallenge" => (2, &[Valid("assertGameId"), Valid("assertDeclineReason")]),
        "openGame" => (2, &[Valid("assertUsername"), Valid("assertGameId")]),
        "playerPerf" => (2, &[Valid("assertUsername"), Valid("assertPerfType")]),
        "crosstable" => (2, &[Valid("assertUsername"), Valid("assertUsername")]),
        "recentGames" => (1, &[Valid("assertUsername"), Valid("assertOptionalFlag")]),
        "sendMessage" => (
            3,
            &[
                Valid("assertUsername"),
                Valid("assertUsername"),
                Valid("assertMessageText"),
            ],
        ),
        "tvChannels" | "stopWatching" => (0, &[]),
        "watch" => (1, &[WatchTarget]),
        "watchBroadcast" => (1, &[Valid("assertLichessId")]),
        "broadcasts" => (0, &[Valid("assertBroadcastQuery")]),
        "broadcastTour" => (1, &[Valid("assertLichessId")]),
        "tournaments" => (1, &[Valid("assertOptionalAccount")]),
        "tournament" => (
            3,
            &[
                Valid("assertTournamentSystem"),
                Valid("assertTournamentId"),
                Valid("assertOptionalAccount"),
                Valid("assertOptionalPage"),
            ],
        ),
        "joinTournament" => (
            3,
            &[
                Valid("assertTournamentSystem"),
                Valid("assertTournamentId"),
                Valid("assertUsername"),
                Valid("assertTournamentPassword"),
            ],
        ),
        "leaveTournament" => (
            3,
            &[
                Valid("assertTournamentSystem"),
                Valid("assertTournamentId"),
                Valid("assertUsername"),
            ],
        ),
        "createTournament" => (2, &[Valid("assertUsername"), Valid("assertNewArena")]),
        "clearAccountData" => (1, &[Valid("assertUsername")]),
        "following" | "usage" | "resetUsage" | "puzzleDaily" | "puzzleDbStatus"
        | "puzzleDbInstall" | "puzzleDbCancel" | "puzzleDbDelete" | "clearVoiceHistory"
        | "library" | "joinedTournaments" => (0, &[]),
        "addFriends" => (1, &[Valid("assertFriendList")]),
        "presence" => (1, &[Valid("assertUsernames")]),
        "puzzleNext" => (1, &[Valid("assertPuzzleRequest")]),
        "puzzleSolve" => (1, &[Valid("assertPuzzleSolve")]),
        "puzzleDashboard" | "stormDashboard" => {
            (2, &[Valid("assertUsername"), Valid("assertDays")])
        }
        "puzzleActivity" => (2, &[Valid("assertUsername"), Valid("assertActivityMax")]),
        "localPuzzles" => (1, &[Valid("assertLocalQuery")]),
        "localLadder" => (1, &[Valid("assertLadderQuery")]),
        "saveRun" => (1, &[Valid("assertRunInput")]),
        "runSummary" => (1, &[Valid("assertRunKind")]),
        "clearRuns" => (0, &[Optional("assertRunKind")]),
        "saveVoiceAttempt" => (1, &[Valid("assertVoiceAttempt")]),
        "updateVoiceAttempt" => (2, &[Valid("assertVoiceId"), Valid("assertVoiceUpdate")]),
        "voiceHistory" => (1, &[Valid("assertVoiceLimit")]),
        "importLibrary" => (1, &[Valid("assertLegacyDocuments")]),
        "studyCommand" => (1, &[Valid("assertStudyCommandShape")]),
        "saveArchivedGame" => (1, &[Valid("assertArchivedGameShape")]),
        "removeArchivedGame" => (1, &[Valid("assertLibraryId")]),
        "addMistakes" => (1, &[Valid("assertReviewKey"), Optional("assertSide")]),
        "answerMistake" => (2, &[Valid("assertLibraryId"), Valid("assertBoolean")]),
        "saveSession" => (
            2,
            &[Valid("assertSessionKind"), Valid("assertSessionObject")],
        ),
        "recordRepertoireMiss" => (2, &[Valid("assertRepertoireKey"), Position]),
        "clearRepertoireMisses" => (1, &[Valid("assertRepertoireKey")]),
        _ => return None,
    };
    Some(Contract { min, checks })
}

/// Checks a method's arguments as `validateCoreArguments` did. `None` when the method has no
/// contract; otherwise the first failing check's message.
pub fn validate_arguments(method: &str, args: &[Value]) -> Option<Result<(), String>> {
    let contract = contract(method)?;
    Some(check_arguments(method, &contract, args))
}

fn check_arguments(method: &str, contract: &Contract, args: &[Value]) -> Result<(), String> {
    if args.len() < contract.min || args.len() > contract.checks.len() {
        return Err(format!("Invalid argument count for {method}."));
    }
    let absent = Value::Null;
    for (index, check) in contract.checks.iter().enumerate() {
        let value = args.get(index).unwrap_or(&absent);
        run_check(*check, value)?;
    }
    if method == "reviewGet" {
        run(
            "assertReviewRequest",
            &[json!({ "fen": args[0], "moves": args[1] })],
        )?;
    }
    Ok(())
}

fn run_check(check: Check, value: &Value) -> Result<(), String> {
    match check {
        Valid(name) => run(name, std::slice::from_ref(value)).map(|_| ()),
        Optional(_) if value.is_null() => Ok(()),
        Optional(name) => run(name, std::slice::from_ref(value)).map(|_| ()),
        WatchTarget => run(
            "assertWatchTarget",
            &[value.clone(), json!(TV_CHANNEL_KEYS)],
        )
        .map(|_| ()),
        Position => run(
            "assertAnalysisRequest",
            &[json!({ "fen": value, "lines": 1 })],
        )
        .map(|_| ()),
        Any => Ok(()),
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
