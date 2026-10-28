//! The core's methods and events.
//!
//! `methods()` is the one table of the methods a transport exposes: name, what it does, and its
//! arguments in order. The CLI lists them and the facade's dispatch is tested against them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::*;

/// One argument of a method.
pub struct Param {
    pub name: &'static str,
    pub optional: bool,
}

/// One method of the core's API.
pub struct Method {
    pub name: &'static str,
    /// What the method does; empty for none.
    pub doc: &'static str,
    pub params: Vec<Param>,
}

fn req(name: &'static str) -> Param {
    Param {
        name,
        optional: false,
    }
}

fn opt(name: &'static str) -> Param {
    Param {
        name,
        optional: true,
    }
}

fn method(name: &'static str, doc: &'static str, params: Vec<Param>) -> Method {
    Method { name, doc, params }
}

/// Every method of `CoreApi`, in declaration order.
pub fn methods() -> Vec<Method> {
    vec![
        method(
            "positionLookup",
            "",
            vec![req("kind"), req("fen"), opt("options")],
        ),
        method(
            "lichessStudies",
            "Studies an account owns or belongs to on Lichess.",
            vec![req("account")],
        ),
        method("lichessStudyChapters", "", vec![req("account"), req("id")]),
        method(
            "syncLichessStudy",
            "Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty).",
            vec![req("request")],
        ),
        method(
            "exportToLichessStudy",
            "",
            vec![req("account"), req("studyId"), req("name"), req("pgn")],
        ),
        method(
            "exportGame",
            "A finished Lichess game's PGN, for the analysis board.",
            vec![req("id")],
        ),
        method(
            "mastersGame",
            "A master game's PGN from the masters database, for the analysis board.",
            vec![req("id")],
        ),
        method(
            "openingDescription",
            "The Wikibooks description of a line of UCI moves from the starting position, or null when it has no page.",
            vec![req("moves")],
        ),
        method(
            "openingPopularity",
            "Month by month, the share of the Lichess games at the position before the line's last move that went on with it.",
            vec![req("moves")],
        ),
        method(
            "cloudEval",
            "Lichess's cached cloud evaluation of a position, or null when it has none.",
            vec![req("fen"), req("lines")],
        ),
        method("loadData", "", vec![]),
        method("saveSettings", "", vec![req("settings")]),
        method("addAccount", "", vec![req("username")]),
        method("logout", "", vec![req("username")]),
        method("logoutAll", "", vec![]),
        method("removeAccount", "", vec![req("username")]),
        method("syncGames", "", vec![opt("username")]),
        method(
            "gamePage",
            "Filtered, bounded library rows; full PGN is fetched separately.",
            vec![req("query")],
        ),
        method("gameLibraryOverview", "", vec![]),
        method(
            "insights",
            "Patterns in an account's synced games, worked out on this computer.",
            vec![req("query")],
        ),
        method("gameRatingHistory", "", vec![req("account")]),
        method(
            "gamePgn",
            "Full PGN of one game; list rows omit it to keep the payload small.",
            vec![req("account"), req("id")],
        ),
        method(
            "cachedProfile",
            "Last profile data saved locally (no network); null fields when never fetched.",
            vec![req("username")],
        ),
        method("profile", "", vec![req("username")]),
        method("ratingHistory", "", vec![req("username")]),
        method(
            "connectLichess",
            "Sign in through the browser. `username` is the account Lichess authorised.",
            vec![opt("look")],
        ),
        method("engineStatus", "", vec![]),
        method(
            "installEngine",
            "Download and verify the latest official Stockfish build into KChess's own folder.\n   * `updated` is false when the installed copy was already the latest and nothing was downloaded.",
            vec![],
        ),
        method(
            "deleteEngine",
            "Delete the downloaded engine; the bundled and chosen ones are never touched.",
            vec![],
        ),
        method("stopEngine", "", vec![]),
        method(
            "bestMove",
            "",
            vec![req("moves"), req("level"), opt("options")],
        ),
        method(
            "startAnalysis",
            "Start analysing a position (replacing any running analysis); updates arrive on `onAnalysis`.",
            vec![req("request")],
        ),
        method(
            "stopAnalysis",
            "Stop the running analysis; with `kill`, also closes the engine.",
            vec![opt("kill")],
        ),
        method(
            "reviewGet",
            "The stored review of these moves from this position, if any.",
            vec![req("fen"), req("moves")],
        ),
        method(
            "reviewRequest",
            "Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess first).\n   * Returns what is stored so far; the rest arrives on `onReviewUpdate`.",
            vec![req("request")],
        ),
        method("reviewCancel", "", vec![req("key")]),
        method("reviewStatus", "", vec![]),
        method(
            "reviewSummaries",
            "Review summaries of up to one history page of games, by game id.",
            vec![req("ids")],
        ),
        method("startOnline", "", vec![req("options")]),
        method(
            "resumeOnline",
            "Reattach to a game in progress on any connected account.",
            vec![],
        ),
        method("cancelOnline", "", vec![]),
        method("playOnline", "", vec![req("id"), req("move")]),
        method("onlineAction", "", vec![req("id"), req("action")]),
        method(
            "onlineChat",
            "The private player chat of the game being played (needs the setting on).",
            vec![req("id")],
        ),
        method("sendChat", "", vec![req("id"), req("room"), req("text")]),
        method(
            "stayConnected",
            "Keep this connected account's Lichess event stream open while no game is being played, so\n   * challenges and tournament pairings arrive; empty closes it.",
            vec![req("account")],
        ),
        method(
            "challenges",
            "Pending challenges to and from the connected accounts.",
            vec![],
        ),
        method("acceptChallenge", "", vec![req("id")]),
        method("declineChallenge", "", vec![req("id"), req("reason")]),
        method(
            "cancelChallenge",
            "Withdraw a challenge you sent.",
            vec![req("id")],
        ),
        method(
            "ongoingGames",
            "Games the connected accounts are playing, most urgent first.",
            vec![],
        ),
        method(
            "openGame",
            "Open one of those games on the online board (a correspondence game, or a live one).",
            vec![req("account"), req("id")],
        ),
        method("playerPerf", "", vec![req("username"), req("perf")]),
        method("crosstable", "", vec![req("a"), req("b")]),
        method(
            "recentGames",
            "A player's latest public games, from their side; `rated` returns more of their rated ones.",
            vec![req("username"), opt("rated")],
        ),
        method(
            "sendMessage",
            "A Lichess private message from one of the connected accounts.",
            vec![req("account"), req("username"), req("text")],
        ),
        method(
            "relation",
            "Whether a connected account follows or blocks a player on Lichess.",
            vec![req("account"), req("username")],
        ),
        method(
            "setRelation",
            "Follow, unfollow, block or unblock a player on Lichess as a connected account.",
            vec![req("account"), req("username"), req("action")],
        ),
        method(
            "profiles",
            "Many players' Lichess profiles at once (fewer details than `profile`), unknown names left out.",
            vec![req("usernames")],
        ),
        method(
            "leaderboard",
            "The top players of a rating category.",
            vec![req("perf")],
        ),
        method(
            "searchPlayers",
            "Players whose names start with `term` (three characters or more).",
            vec![req("term")],
        ),
        method(
            "activity",
            "A player's recent Lichess activity, by day.",
            vec![req("username")],
        ),
        method(
            "timeline",
            "What the players a connected account follows did lately.",
            vec![req("account")],
        ),
        method(
            "importGame",
            "Upload a PGN game to Lichess (as `account`, or anonymously when empty) for its page.",
            vec![req("account"), req("pgn")],
        ),
        method(
            "bookmarkedGames",
            "The games a connected account bookmarked on Lichess, from its side.",
            vec![req("account")],
        ),
        method(
            "tvChannels",
            "Lichess TV channels and who is on each.",
            vec![],
        ),
        method(
            "watch",
            "Watch a TV channel or any game by id (replacing what was watched); frames on `onWatch`.",
            vec![req("target")],
        ),
        method(
            "watchBroadcast",
            "Follow a broadcast round's live PGN; updates on `onBroadcast`.",
            vec![req("roundId")],
        ),
        method("stopWatching", "", vec![]),
        method("broadcasts", "", vec![opt("query")]),
        method("broadcastTour", "", vec![req("id")]),
        method(
            "tournaments",
            "Current arenas, and the Swiss events of the account's teams.",
            vec![req("account")],
        ),
        method(
            "tournament",
            "One tournament; `page` picks the leaderboard page (10 players each, arenas only).",
            vec![req("system"), req("id"), req("account"), opt("page")],
        ),
        method(
            "joinTournament",
            "",
            vec![req("system"), req("id"), req("account"), opt("password")],
        ),
        method(
            "leaveTournament",
            "",
            vec![req("system"), req("id"), req("account")],
        ),
        method(
            "createTournament",
            "Creates an arena on Lichess run by the account.",
            vec![req("account"), req("arena")],
        ),
        method(
            "clearAccountData",
            "Delete the games and cached profile downloaded for one account, keeping the account.",
            vec![req("username")],
        ),
        method(
            "following",
            "Players the connected accounts follow on Lichess (needs the follow permission).",
            vec![],
        ),
        method(
            "addFriends",
            "Add players as friends without downloading their games.",
            vec![req("usernames")],
        ),
        method(
            "usage",
            "Data downloaded from Lichess per account, and what is stored locally.",
            vec![],
        ),
        method("resetUsage", "", vec![]),
        method(
            "presence",
            "Online/playing/signal of the given users, for the game in progress.",
            vec![req("usernames")],
        ),
        method(
            "puzzleNext",
            "Lichess puzzle training. Rated results change the account's Lichess puzzle rating.",
            vec![req("request")],
        ),
        method(
            "puzzleSolve",
            "Report a puzzle result to Lichess (needs `puzzle:write`).",
            vec![req("request")],
        ),
        method(
            "puzzleDaily",
            "The daily puzzle; public, nothing is recorded.",
            vec![],
        ),
        method(
            "puzzleDashboard",
            "Read-only: the account's puzzle dashboard.",
            vec![req("account"), req("days")],
        ),
        method(
            "puzzleActivity",
            "Read-only: the account's most recent puzzle attempts.",
            vec![req("account"), req("max")],
        ),
        method(
            "stormDashboard",
            "Read-only: anyone's public Storm dashboard.",
            vec![req("username"), req("days")],
        ),
        method(
            "puzzleDbStatus",
            "Local puzzle database (downloaded from database.lichess.org).",
            vec![],
        ),
        method("puzzleDbInstall", "", vec![]),
        method("puzzleDbCancel", "", vec![]),
        method("puzzleDbDelete", "", vec![]),
        method("localPuzzles", "", vec![req("query")]),
        method(
            "localLadder",
            "Puzzles of rising difficulty, for Storm, Streak and Rush.",
            vec![req("query")],
        ),
        method(
            "recordPuzzleAttempt",
            "Local training log (Practice, Downloaded and Rated attempts), kept in the puzzle database. It never leaves this computer.",
            vec![req("attempt")],
        ),
        method("puzzleProgress", "", vec![req("query")]),
        method(
            "saveRun",
            "Local scores (Storm, Streak, Rush and the practice drills). They never leave this computer.",
            vec![req("run")],
        ),
        method("runSummary", "", vec![req("kind")]),
        method("clearRuns", "", vec![opt("kind")]),
        method(
            "saveVoiceAttempt",
            "Local log of voice input (Settings › Voice). It never leaves this computer unless exported.",
            vec![req("attempt")],
        ),
        method("updateVoiceAttempt", "", vec![req("id"), req("update")]),
        method("voiceHistory", "Newest first.", vec![req("limit")]),
        method("clearVoiceHistory", "", vec![]),
        method(
            "voiceStart",
            "Offline speech recognition: listens for these phrases, discarding any utterance in progress.",
            vec![req("grammar"), req("sampleRate")],
        ),
        method(
            "voiceAccept",
            "Base64 16-bit little-endian mono PCM at the started sample rate.",
            vec![req("audio")],
        ),
        method(
            "voiceFinish",
            "Ends the utterance (push-to-talk released).",
            vec![],
        ),
        method(
            "voiceStop",
            "Releases the recognizer and its model.",
            vec![],
        ),
        method(
            "library",
            "The local library as saved: studies, played games, drills, sessions and training notes.",
            vec![],
        ),
        method(
            "importLibrary",
            "Take over the documents an earlier release kept in the renderer; runs once.",
            vec![req("documents")],
        ),
        method(
            "studyCommand",
            "",
            vec![req("command"), opt("incremental"), opt("basedOn")],
        ),
        method(
            "saveArchivedGame",
            "Add or replace a played game; it becomes the most recent.",
            vec![req("game")],
        ),
        method("removeArchivedGame", "", vec![req("id")]),
        method(
            "addMistakes",
            "Drills from a stored review's mistakes (one side's, when given); `added` counts new ones.",
            vec![req("reviewKey"), opt("color")],
        ),
        method("answerMistake", "", vec![req("id"), req("solved")]),
        method(
            "mistakeGames",
            "The reviewed games with positions in practice, most recently played first.",
            vec![],
        ),
        method(
            "saveSession",
            "Keep an unfinished game or analysis so it can be resumed.",
            vec![req("kind"), req("session")],
        ),
        method(
            "joinedTournaments",
            "Tournaments joined from KChess that have not ended.",
            vec![],
        ),
        method("recordRepertoireMiss", "", vec![req("key"), req("fen")]),
        method("clearRepertoireMisses", "", vec![req("key")]),
        method(
            "customThemes",
            "The user's own app themes from the profile's themes folder, with a line per file that could not be used.",
            vec![],
        ),
        method(
            "viewPreferences",
            "How each page was last left (tabs, filters, toggles), by `kchess:` preference name.",
            vec![],
        ),
        method(
            "setViewPreference",
            "Remember a view preference for every KChess app on this profile; null forgets it.",
            vec![req("key"), req("value")],
        ),
    ]
}

/// Everything the core reports without being asked, keyed like the desktop's IPC events.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CoreEvents {
    #[serde(rename = "online:state")]
    pub online_state: OnlineConnection,
    #[serde(rename = "online:event")]
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
    /// Another window (or this one) changed part of the profile; refetch what is shown.
    #[serde(rename = "profile:changed")]
    pub profile_changed: ProfileChange,
    /// Which window raises system notifications now; the others stay quiet.
    #[serde(rename = "alerts:owner")]
    pub alerts_owner: AlertOwner,
    /// The client connected to the shared core again after it stopped (`docs/shared-core.md`):
    /// what this window had started there (an analysis, a watch, voice input) ended with the
    /// old connection, and its owners start it again. Sent by the client, not the core.
    #[serde(rename = "core:reconnected")]
    pub core_reconnected: (),
}

#[cfg(test)]
mod tests {
    use super::methods;

    #[test]
    fn method_names_are_unique() {
        let methods = methods();
        let mut names: Vec<&str> = methods.iter().map(|m| m.name).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "method names are unique");
    }
}
