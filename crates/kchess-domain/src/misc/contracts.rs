//! The argument contracts of the core's methods (`CORE_CONTRACTS`) and of the desktop's IPC
//! invocations (`IPC_CONTRACTS`): how many arguments a method takes, and one validator per
//! argument, run before the method does anything. Desktop methods check their own rows first and
//! fall back to the core's, so an IPC method with the same name as a core method (`saveSettings`)
//! takes the desktop's check.
//!
//! The tables are the single source. `kchess-contracts` generates their arities for TypeScript
//! (`ts/generated/arity.ts`), and the frontends check calls through `validateCoreArguments` and
//! `validateIpcArguments` on the rules binding. A JSON `null` stands for `undefined` where the
//! facade passes JSON arguments, since JSON cannot carry `undefined`.

use serde_json::{Value, json};

use super::engine::J;
use super::{call, documents, validate};

/// One argument's validator.
#[derive(Clone, Copy)]
enum Check {
    /// A validator by its TypeScript name.
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
    /// `IPC_CONTRACTS`: the desktop's invocations (its own rows, then the core's).
    Ipc,
}

/// The channels `assertWatchTarget` accepts (`TV_CHANNEL_KEYS`, `crates/kchess-wasm/js/tvChannels.ts`).
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
    ("gameRatingHistory", 1, &[Valid("assertUsername")]),
    (
        "gamePgn",
        2,
        &[Valid("assertUsername"), Valid("assertGameId")],
    ),
    ("cachedProfile", 1, &[Valid("assertUsername")]),
    ("profile", 1, &[Valid("assertUsername")]),
    ("ratingHistory", 1, &[Valid("assertUsername")]),
    ("connectLichess", 0, &[Any]),
    ("engineStatus", 0, &[]),
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
    ("stopAnalysis", 0, &[]),
    ("reviewGet", 2, &[Position, Valid("assertMoves")]),
    ("reviewRequest", 1, &[Valid("assertReviewRequest")]),
    ("reviewCancel", 1, &[Valid("assertReviewKey")]),
    ("reviewStatus", 0, &[]),
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
        &[Valid("assertUsername"), Valid("assertOptionalFlag")],
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
    ("tvChannels", 0, &[]),
    ("stopWatching", 0, &[]),
    ("watch", 1, &[WatchTarget]),
    ("watchBroadcast", 1, &[Valid("assertLichessId")]),
    ("broadcasts", 0, &[Valid("assertBroadcastQuery")]),
    ("broadcastTour", 1, &[Valid("assertLichessId")]),
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
    ("library", 0, &[]),
    ("importLibrary", 1, &[Valid("assertLegacyDocuments")]),
    ("studyCommand", 1, &[Valid("assertStudyCommandShape")]),
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
];

/// The desktop's own invocations (`IPC_CHANNELS` that are not core methods, and `saveSettings`,
/// whose desktop settings differ from the core's).
const DESKTOP: &[Row] = &[
    ("completeQuit", 1, &[Valid("assertUuid")]),
    (
        "recordPerformance",
        2,
        &[Valid("assertPerformanceName"), Valid("assertTimingValue")],
    ),
    ("reportRendererError", 1, &[Valid("assertRendererError")]),
    ("saveExport", 1, &[Valid("assertExport")]),
    ("exportDiagnostics", 0, &[]),
    ("windowMinimize", 0, &[]),
    ("windowToggleMaximize", 0, &[]),
    ("windowClose", 0, &[]),
    ("windowIsMaximized", 0, &[]),
    ("appUpdateStatus", 0, &[]),
    ("checkAppUpdate", 0, &[]),
    ("downloadAppUpdate", 0, &[]),
    ("installAppUpdate", 0, &[]),
    ("openAppReleases", 0, &[]),
    ("chooseEngine", 0, &[]),
    ("notify", 1, &[Valid("assertNotification")]),
    ("loadThemes", 0, &[]),
    ("openThemesFolder", 0, &[]),
    ("openNotificationSettings", 0, &[]),
    ("microphoneAccess", 1, &[Valid("assertBoolean")]),
    ("ensureVoiceModel", 0, &[]),
    ("voiceModelStatus", 0, &[]),
    ("openMicrophoneSettings", 0, &[]),
    ("exportVoiceHistory", 0, &[]),
    ("saveSettings", 1, &[Valid("assertSettings")]),
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
        Scope::Ipc => find(DESKTOP).or_else(|| find(CORE)),
    }
}

/// Every method of `scope` with its fewest and most arguments, sorted by name.
pub fn arities(scope: Scope) -> Vec<(&'static str, usize, usize)> {
    let rows: Vec<&Row> = match scope {
        Scope::Core => CORE.iter().collect(),
        Scope::Ipc => DESKTOP
            .iter()
            .chain(
                CORE.iter()
                    .filter(|(name, ..)| !DESKTOP.iter().any(|(d, ..)| d == name)),
            )
            .collect(),
    };
    let mut out: Vec<(&'static str, usize, usize)> = rows
        .into_iter()
        .map(|(name, min, checks)| (*name, *min, checks.len()))
        .collect();
    out.sort_by_key(|(name, ..)| *name);
    out
}

/// A validator or decoder: its arguments in, its normalized value or message out.
type Run = fn(Vec<J>) -> Result<J, String>;

/// The validator named `name`: a value validator, or a document decoder.
fn resolve(name: &str) -> Option<Run> {
    validate::validator(name).or_else(|| documents::handler(name))
}

/// Runs validator `name` on `args`; a rejection is its message.
fn apply(name: &str, args: Vec<J>) -> Result<J, String> {
    match resolve(name) {
        Some(validator) => validator(args),
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

/// Whether `method` is one of the argument-check calls (`validateCoreArguments`,
/// `validateIpcArguments`), which take `[method, args]`.
pub fn is_call(method: &str) -> bool {
    matches!(method, "validateCoreArguments" | "validateIpcArguments")
}

/// The argument-check calls. `values` are `[method, args]`, with `undefined` kept where the
/// frontend sent it. A valid call returns `null`.
pub fn argument_call(method: &str, values: &[J]) -> Result<J, String> {
    let scope = match method {
        "validateIpcArguments" => Scope::Ipc,
        _ => Scope::Core,
    };
    match values {
        [J::Str(name), J::Arr(list)] => check(scope, name, list).map(|()| J::Null),
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
        for (method, _, checks) in CORE.iter().chain(DESKTOP) {
            for check in checks.iter() {
                if let Valid(name) | Optional(name) = check {
                    assert!(
                        resolve(name).is_some(),
                        "{method} names no validator {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn desktop_rows_override_core_rows_of_the_same_name() {
        let (_, checks) = row(Scope::Ipc, "saveSettings").expect("saveSettings");
        assert!(matches!(checks[0], Valid("assertSettings")));
        let (_, checks) = row(Scope::Core, "saveSettings").expect("saveSettings");
        assert!(matches!(checks[0], Valid("assertCoreSettings")));
    }

    #[test]
    fn excess_and_missing_arguments_are_rejected_before_any_check() {
        let excess = vec![J::Undef; 3];
        assert_eq!(
            check(Scope::Ipc, "completeQuit", &excess),
            Err("Invalid argument count for completeQuit.".into())
        );
        assert_eq!(
            check(Scope::Ipc, "recordPerformance", &[]),
            Err("Invalid argument count for recordPerformance.".into())
        );
        assert_eq!(
            check(Scope::Ipc, "unknownMethod", &[]),
            Err("Invalid argument count for unknownMethod.".into())
        );
    }
}
