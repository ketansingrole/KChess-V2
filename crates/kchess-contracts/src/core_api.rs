//! `crates/kchess-contracts/ts/core.ts`: the core's methods and events.
//!
//! `METHODS` is the one table. `CoreApi` (the interface frontends call), `CoreMethod` and
//! `CORE_METHODS` (the list a transport exposes) are all rendered from it, so they cannot drift
//! apart. Argument and result types are ts-rs names of the contract types; the few that ts-rs
//! cannot name (the generic `saveSession`, object literals, `true | NeedsReconnect`) are text.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::{Config, TS};

use crate::generate::Item;
use crate::library::{
    ArchivedGame, JoinedTournament, LibrarySnapshot, MistakeExercise, StudyCommandResult,
};
use crate::types::*;

/// One argument of a method.
pub struct Param {
    pub name: &'static str,
    pub ts: String,
    pub optional: bool,
}

/// One method of `CoreApi`.
pub struct Method {
    pub name: &'static str,
    /// JSDoc of the method; empty for none.
    pub doc: &'static str,
    /// Generic parameters, e.g. `<K extends SessionKind>`.
    pub generics: &'static str,
    pub params: Vec<Param>,
    /// The type inside `Promise<…>`.
    pub result: String,
}

/// The name ts-rs gives `T` in TypeScript (`PositionLookup`, `LichessStudy[]`, …).
fn t<T: TS + ?Sized>() -> String {
    T::name(&Config::new())
}

fn req(name: &'static str, ts: impl Into<String>) -> Param {
    Param {
        name,
        ts: ts.into(),
        optional: false,
    }
}

fn opt(name: &'static str, ts: impl Into<String>) -> Param {
    Param {
        name,
        ts: ts.into(),
        optional: true,
    }
}

/// `T | NeedsReconnect`, the result of every call that needs a Lichess permission.
fn or_reconnect(ts: impl Into<String>) -> String {
    format!("{} | {}", ts.into(), t::<NeedsReconnect>())
}

fn array(ts: impl Into<String>) -> String {
    format!("{}[]", ts.into())
}

fn method(
    name: &'static str,
    doc: &'static str,
    params: Vec<Param>,
    result: impl Into<String>,
) -> Method {
    Method {
        name,
        doc,
        generics: "",
        params,
        result: result.into(),
    }
}

/// Every method of `CoreApi`, in declaration order.
pub fn methods() -> Vec<Method> {
    let string = "string";
    let number = "number";
    let boolean = "boolean";
    let void = "void";
    vec![
        method(
            "positionLookup",
            "",
            vec![
                req("kind", t::<PositionLookupKind>()),
                req("fen", string),
                opt("options", t::<LookupOptions>()),
            ],
            t::<PositionLookup>(),
        ),
        method(
            "lichessStudies",
            "Studies an account owns or belongs to on Lichess.",
            vec![req("account", string)],
            or_reconnect(array(t::<LichessStudy>())),
        ),
        method(
            "lichessStudyChapters",
            "",
            vec![req("account", string), req("id", string)],
            or_reconnect(array(t::<LichessStudyChapter>())),
        ),
        method(
            "syncLichessStudy",
            "Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty).",
            vec![req("request", t::<StudySyncRequest>())],
            or_reconnect(array(t::<LichessStudyChapter>())),
        ),
        method(
            "exportToLichessStudy",
            "",
            vec![
                req("account", string),
                req("studyId", string),
                req("name", string),
                req("pgn", string),
            ],
            format!("{{ id: string }} | {}", t::<NeedsReconnect>()),
        ),
        method(
            "exportGame",
            "A finished Lichess game's PGN, for the analysis board.",
            vec![req("id", string)],
            string,
        ),
        method(
            "mastersGame",
            "A master game's PGN from the masters database, for the analysis board.",
            vec![req("id", string)],
            string,
        ),
        method(
            "cloudEval",
            "Lichess's cached cloud evaluation of a position, or null when it has none.",
            vec![req("fen", string), req("lines", number)],
            format!("{} | null", t::<CloudEval>()),
        ),
        method("loadData", "", vec![], "CoreData"),
        method(
            "saveSettings",
            "",
            vec![req("settings", t::<CoreSettings>())],
            t::<CoreSettings>(),
        ),
        method("addAccount", "", vec![req("username", string)], "CoreData"),
        method("logout", "", vec![req("username", string)], "CoreData"),
        method("logoutAll", "", vec![], "CoreData"),
        method(
            "removeAccount",
            "",
            vec![req("username", string)],
            "CoreData",
        ),
        method("syncGames", "", vec![opt("username", string)], "CoreData"),
        method(
            "gamePage",
            "Filtered, bounded library rows; full PGN is fetched separately.",
            vec![req("query", t::<GamePageQuery>())],
            t::<GamePage>(),
        ),
        method(
            "gameLibraryOverview",
            "",
            vec![],
            t::<GameLibraryOverview>(),
        ),
        method(
            "insights",
            "Patterns in an account's synced games, worked out on this computer.",
            vec![req("query", t::<InsightsQuery>())],
            t::<InsightsReport>(),
        ),
        method(
            "gameRatingHistory",
            "",
            vec![req("account", string)],
            "LichessRatingHistory",
        ),
        method(
            "gamePgn",
            "Full PGN of one game; list rows omit it to keep the payload small.",
            vec![req("account", string), req("id", string)],
            format!("{string} | null"),
        ),
        method(
            "cachedProfile",
            "Last profile data saved locally (no network); null fields when never fetched.",
            vec![req("username", string)],
            format!(
                "{{\n    profile: {} | null\n    ratingHistory: {} | null\n    profileFetchedAt?: number\n  }}",
                "LichessUser", "LichessRatingHistory"
            ),
        ),
        method("profile", "", vec![req("username", string)], "LichessUser"),
        method(
            "ratingHistory",
            "",
            vec![req("username", string)],
            "LichessRatingHistory",
        ),
        method(
            "connectLichess",
            "Sign in through the browser. `username` is the account Lichess authorised.",
            vec![opt("look", t::<OAuthPageLook>())],
            format!("{{ data: {}; username: string }}", "CoreData"),
        ),
        method("engineStatus", "", vec![], t::<EngineStatus>()),
        method(
            "installEngine",
            "Download and verify the latest official Stockfish build into KChess's own folder.\n   * `updated` is false when the installed copy was already the latest and nothing was downloaded.",
            vec![],
            "{ path: string; version: string; updated: boolean }",
        ),
        method(
            "deleteEngine",
            "Delete the downloaded engine; the bundled and chosen ones are never touched.",
            vec![],
            void,
        ),
        method("stopEngine", "", vec![], void),
        method(
            "bestMove",
            "",
            vec![
                req("moves", array(string)),
                req("level", t::<EngineLevel>()),
                opt("options", t::<BestMoveOptions>()),
            ],
            string,
        ),
        method(
            "startAnalysis",
            "Start analysing a position (replacing any running analysis); updates arrive on `onAnalysis`.",
            vec![req("request", t::<AnalysisRequest>())],
            number,
        ),
        method("stopAnalysis", "", vec![], void),
        method(
            "reviewGet",
            "The stored review of these moves from this position, if any.",
            vec![req("fen", string), req("moves", array(string))],
            format!("{} | null", t::<StoredReview>()),
        ),
        method(
            "reviewRequest",
            "Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess first).\n   * Returns what is stored so far; the rest arrives on `onReviewUpdate`.",
            vec![req("request", t::<ReviewRequest>())],
            format!("{} | null", t::<StoredReview>()),
        ),
        method("reviewCancel", "", vec![req("key", string)], void),
        method("reviewStatus", "", vec![], t::<ReviewStatus>()),
        method(
            "reviewSummaries",
            "Review summaries of up to one history page of games, by game id.",
            vec![req("ids", array(string))],
            format!("Record<string, {}>", t::<ReviewSummary>()),
        ),
        method(
            "startOnline",
            "",
            vec![req("options", t::<OnlineOptions>())],
            "{ id?: string; url?: string; seeking?: boolean }",
        ),
        method(
            "resumeOnline",
            "Reattach to a game in progress on any connected account.",
            vec![],
            "{ id: string; account: string } | null",
        ),
        method("cancelOnline", "", vec![], void),
        method(
            "playOnline",
            "",
            vec![req("id", string), req("move", string)],
            void,
        ),
        method(
            "onlineAction",
            "",
            vec![req("id", string), req("action", t::<OnlineAction>())],
            void,
        ),
        method(
            "onlineChat",
            "The private player chat of the game being played (needs the setting on).",
            vec![req("id", string)],
            array(t::<ChatLine>()),
        ),
        method(
            "sendChat",
            "",
            vec![
                req("id", string),
                req("room", t::<ChatRoom>()),
                req("text", string),
            ],
            void,
        ),
        method(
            "stayConnected",
            "Keep this connected account's Lichess event stream open while no game is being played, so\n   * challenges and tournament pairings arrive; empty closes it.",
            vec![req("account", string)],
            void,
        ),
        method(
            "challenges",
            "Pending challenges to and from the connected accounts.",
            vec![],
            array(t::<ChallengeInfo>()),
        ),
        method("acceptChallenge", "", vec![req("id", string)], void),
        method(
            "declineChallenge",
            "",
            vec![req("id", string), req("reason", t::<DeclineReason>())],
            void,
        ),
        method(
            "cancelChallenge",
            "Withdraw a challenge you sent.",
            vec![req("id", string)],
            void,
        ),
        method(
            "ongoingGames",
            "Games the connected accounts are playing, most urgent first.",
            vec![],
            array(t::<OngoingGame>()),
        ),
        method(
            "openGame",
            "Open one of those games on the online board (a correspondence game, or a live one).",
            vec![req("account", string), req("id", string)],
            void,
        ),
        method(
            "playerPerf",
            "",
            vec![req("username", string), req("perf", t::<PerfType>())],
            t::<PerfStats>(),
        ),
        method(
            "crosstable",
            "",
            vec![req("a", string), req("b", string)],
            t::<Crosstable>(),
        ),
        method(
            "recentGames",
            "A player's latest public games, from their side; `rated` returns more of their rated ones.",
            vec![req("username", string), opt("rated", boolean)],
            array(t::<LichessGame>()),
        ),
        method(
            "sendMessage",
            "A Lichess private message from one of the connected accounts.",
            vec![
                req("account", string),
                req("username", string),
                req("text", string),
            ],
            format!("{{ sent: true }} | {}", t::<NeedsReconnect>()),
        ),
        method(
            "tvChannels",
            "Lichess TV channels and who is on each.",
            vec![],
            array(t::<TvChannel>()),
        ),
        method(
            "watch",
            "Watch a TV channel or any game by id (replacing what was watched); frames on `onWatch`.",
            vec![req("target", "{ channel: string } | { gameId: string }")],
            number,
        ),
        method(
            "watchBroadcast",
            "Follow a broadcast round's live PGN; updates on `onBroadcast`.",
            vec![req("roundId", string)],
            number,
        ),
        method("stopWatching", "", vec![], void),
        method(
            "broadcasts",
            "",
            vec![opt("query", string)],
            array(t::<BroadcastSummary>()),
        ),
        method(
            "broadcastTour",
            "",
            vec![req("id", string)],
            t::<BroadcastTourDetail>(),
        ),
        method(
            "tournaments",
            "Current arenas, and the Swiss events of the account's teams.",
            vec![req("account", string)],
            t::<TournamentList>(),
        ),
        method(
            "tournament",
            "One tournament; `page` picks the leaderboard page (10 players each, arenas only).",
            vec![
                req("system", t::<TournamentSystem>()),
                req("id", string),
                req("account", string),
                opt("page", number),
            ],
            t::<TournamentDetail>(),
        ),
        method(
            "joinTournament",
            "",
            vec![
                req("system", t::<TournamentSystem>()),
                req("id", string),
                req("account", string),
                opt("password", string),
            ],
            format!("true | {}", t::<NeedsReconnect>()),
        ),
        method(
            "leaveTournament",
            "",
            vec![
                req("system", t::<TournamentSystem>()),
                req("id", string),
                req("account", string),
            ],
            format!("true | {}", t::<NeedsReconnect>()),
        ),
        method(
            "createTournament",
            "Creates an arena on Lichess run by the account.",
            vec![req("account", string), req("arena", t::<NewArena>())],
            format!("{} | {}", t::<TournamentSummary>(), t::<NeedsReconnect>()),
        ),
        method(
            "clearAccountData",
            "Delete the games and cached profile downloaded for one account, keeping the account.",
            vec![req("username", string)],
            "CoreData",
        ),
        method(
            "following",
            "Players the connected accounts follow on Lichess (needs the follow permission).",
            vec![],
            t::<FollowingReport>(),
        ),
        method(
            "addFriends",
            "Add players as friends without downloading their games.",
            vec![req("usernames", array(string))],
            "CoreData",
        ),
        method(
            "usage",
            "Data downloaded from Lichess per account, and what is stored locally.",
            vec![],
            t::<UsageReport>(),
        ),
        method("resetUsage", "", vec![], void),
        method(
            "presence",
            "Online/playing/signal of the given users, for the game in progress.",
            vec![req("usernames", array(string))],
            t::<PresenceReport>(),
        ),
        method(
            "puzzleNext",
            "Lichess puzzle training. Rated results change the account's Lichess puzzle rating.",
            vec![req("request", t::<PuzzleRequest>())],
            or_reconnect(t::<PuzzleDraw>()),
        ),
        method(
            "puzzleSolve",
            "Report a puzzle result to Lichess (needs `puzzle:write`).",
            vec![req("request", t::<PuzzleSolveRequest>())],
            or_reconnect(t::<PuzzleSolveResult>()),
        ),
        method(
            "puzzleDaily",
            "The daily puzzle; public, nothing is recorded.",
            vec![],
            t::<Puzzle>(),
        ),
        method(
            "puzzleDashboard",
            "Read-only: the account's puzzle dashboard.",
            vec![req("account", string), req("days", number)],
            or_reconnect(t::<PuzzleDashboard>()),
        ),
        method(
            "puzzleActivity",
            "Read-only: the account's most recent puzzle attempts.",
            vec![req("account", string), req("max", number)],
            or_reconnect(array(t::<PuzzleActivityEntry>())),
        ),
        method(
            "stormDashboard",
            "Read-only: anyone's public Storm dashboard.",
            vec![req("username", string), req("days", number)],
            t::<StormDashboard>(),
        ),
        method(
            "puzzleDbStatus",
            "Local puzzle database (downloaded from database.lichess.org).",
            vec![],
            t::<PuzzleDbStatus>(),
        ),
        method("puzzleDbInstall", "", vec![], t::<PuzzleDbStatus>()),
        method("puzzleDbCancel", "", vec![], void),
        method("puzzleDbDelete", "", vec![], t::<PuzzleDbStatus>()),
        method(
            "localPuzzles",
            "",
            vec![req("query", t::<LocalPuzzleQuery>())],
            array(t::<Puzzle>()),
        ),
        method(
            "localLadder",
            "Puzzles of rising difficulty, for Storm, Streak and Rush.",
            vec![req("query", t::<LocalLadderQuery>())],
            array(t::<Puzzle>()),
        ),
        method(
            "saveRun",
            "Local scores (Storm, Streak, Rush and the practice drills). They never leave this computer.",
            vec![req("run", t::<RunInput>())],
            t::<RunSaved>(),
        ),
        method(
            "runSummary",
            "",
            vec![req("kind", t::<RunKind>())],
            t::<RunSummary>(),
        ),
        method("clearRuns", "", vec![opt("kind", t::<RunKind>())], void),
        method(
            "saveVoiceAttempt",
            "Local log of voice input (Settings › Voice). It never leaves this computer unless exported.",
            vec![req("attempt", t::<VoiceAttemptInput>())],
            number,
        ),
        method(
            "updateVoiceAttempt",
            "",
            vec![req("id", number), req("update", t::<VoiceAttemptUpdate>())],
            void,
        ),
        method(
            "voiceHistory",
            "Newest first.",
            vec![req("limit", number)],
            array(t::<VoiceAttempt>()),
        ),
        method("clearVoiceHistory", "", vec![], void),
        method(
            "library",
            "The local library as saved: studies, played games, drills, sessions and training notes.",
            vec![],
            t::<LibrarySnapshot>(),
        ),
        method(
            "importLibrary",
            "Take over the documents an earlier release kept in the renderer; runs once.",
            vec![req("documents", "LegacyDocuments")],
            t::<LibrarySnapshot>(),
        ),
        method(
            "studyCommand",
            "",
            vec![req("command", "StudyCommand")],
            t::<StudyCommandResult>(),
        ),
        method(
            "saveArchivedGame",
            "Add or replace a played game; it becomes the most recent.",
            vec![req("game", t::<ArchivedGame>())],
            void,
        ),
        method("removeArchivedGame", "", vec![req("id", string)], void),
        method(
            "addMistakes",
            "Drills from a stored review's mistakes (one side's, when given); `added` counts new ones.",
            vec![req("reviewKey", string), opt("color", t::<PlayerColor>())],
            format!(
                "{{ added: number; items: {} }}",
                array(t::<MistakeExercise>())
            ),
        ),
        method(
            "answerMistake",
            "",
            vec![req("id", string), req("solved", boolean)],
            array(t::<MistakeExercise>()),
        ),
        Method {
            name: "saveSession",
            doc: "Keep an unfinished game or analysis so it can be resumed.",
            generics: "<K extends SessionKind>",
            params: vec![req("kind", "K"), req("session", "SessionDocuments[K]")],
            result: void.to_string(),
        },
        method(
            "joinedTournaments",
            "Tournaments joined from KChess that have not ended.",
            vec![],
            array(t::<JoinedTournament>()),
        ),
        method(
            "recordRepertoireMiss",
            "",
            vec![req("key", string), req("fen", string)],
            "RepertoireMisses",
        ),
        method(
            "clearRepertoireMisses",
            "",
            vec![req("key", string)],
            "RepertoireMisses",
        ),
    ]
}

/// Everything the core reports without being asked, keyed like the desktop's IPC events.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct CoreEvents {
    #[serde(rename = "online:state")]
    pub online_state: OnlineConnection,
    #[serde(rename = "online:event")]
    #[ts(type = "OnlineEvent")]
    pub online_event: Value,
    #[serde(rename = "online:error")]
    pub online_error: String,
    #[serde(rename = "puzzledb:progress")]
    pub puzzledb_progress: PuzzleDbProgress,
    #[serde(rename = "engine:analysis")]
    pub engine_analysis: AnalysisUpdate,
    #[serde(rename = "review:update")]
    pub review_update: ReviewUpdate,
    #[serde(rename = "review:status")]
    pub review_status: ReviewStatus,
    #[serde(rename = "challenges:update")]
    pub challenges_update: Vec<ChallengeInfo>,
    #[serde(rename = "online:lobby")]
    pub online_lobby: LobbyState,
    #[serde(rename = "online:ongoing-changed")]
    pub online_ongoing_changed: (),
    #[serde(rename = "watch:state")]
    pub watch_state: WatchState,
    #[serde(rename = "watch:frame")]
    pub watch_frame: WatchFrame,
    #[serde(rename = "watch:broadcast")]
    pub watch_broadcast: BroadcastUpdate,
    /// A challenge that was not pending before; frontends decide how to alert.
    #[serde(rename = "challenge:received")]
    pub challenge_received: ChallengeInfo,
    #[serde(rename = "settings:saved")]
    pub settings_saved: CoreSettings,
}

/// Renders the `CoreApi` interface from `methods()`.
fn core_api_text() -> String {
    let mut out = String::from("export interface CoreApi {\n");
    for m in methods() {
        if !m.doc.is_empty() {
            out.push_str("  /**\n");
            for line in m.doc.split('\n') {
                let text = line.trim().trim_start_matches('*').trim();
                out.push_str(&format!("   * {text}\n"));
            }
            out.push_str("   */\n");
        }
        let params = m
            .params
            .iter()
            .map(|p| format!("{}{}: {}", p.name, if p.optional { "?" } else { "" }, p.ts))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "  {}{}({}): Promise<{}>\n",
            m.name, m.generics, params, m.result
        ));
    }
    out.push('}');
    out
}

/// `CORE_METHODS`: the method names of `methods()`, in order.
fn core_methods_text() -> String {
    let names = methods()
        .iter()
        .map(|m| format!("  '{}',", m.name))
        .collect::<Vec<_>>()
        .join("\n");
    format!("export const CORE_METHODS: CoreMethod[] = [\n{names}\n]")
}

/// Every declaration of `core.ts`.
pub fn items(cfg: &Config) -> Vec<Item> {
    vec![
        Item::of::<CoreEvents>(cfg),
        Item::Raw {
            name: "CoreApi".into(),
            text: core_api_text(),
        },
        Item::Raw {
            name: "CoreMethod".into(),
            text: "export type CoreMethod = keyof CoreApi;".into(),
        },
        Item::Raw {
            name: "CORE_METHODS".into(),
            text: core_methods_text(),
        },
    ]
}
