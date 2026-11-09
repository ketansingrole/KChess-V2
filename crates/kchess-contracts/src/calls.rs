//! Typed core calls: each names a method, its arguments in order and the shape of its answer, so
//! a frontend that makes one cannot misspell the method, send arguments of the wrong shape or
//! misread what comes back. A frontend still reaches the core by method name (`AppCore::request`
//! in the app sends `METHOD` with `args()` and decodes the answer with `decode`).
//!
//! `CALLS` lists every typed call's method and argument count; the core's tests check each
//! against the facade's methods and the arities the validator accepts
//! (`crates/kchess-core/tests/core/facade_dispatch.rs`).

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::CoreError;
use crate::types::{CustomThemeReport, Settings};

/// One core method as a typed call.
pub trait CoreCall {
    /// The method's name, as `CoreApi` (or the facade, for its own methods) lists it.
    const METHOD: &'static str;
    /// What the method answers.
    type Answer: DeserializeOwned;
    /// The arguments, in the method's order.
    fn args(self) -> Vec<Value>;
}

/// The core's answer to `C` in its shape, or an error that names the method.
pub fn decode<C: CoreCall>(answer: Value) -> Result<C::Answer, CoreError> {
    serde_json::from_value(answer).map_err(|cause| {
        CoreError::new(format!(
            "The core's answer to {} could not be read: {cause}",
            C::METHOD
        ))
    })
}

/// `args` without the `null`s at its end: an optional argument not given is left out, as callers
/// sent it before typed calls, rather than sent as `null` (which the validator reads alike).
fn without_trailing_nulls(mut args: Vec<Value>) -> Vec<Value> {
    while args.last().is_some_and(Value::is_null) {
        args.pop();
    }
    args
}

/// An argument as JSON. The contract types always serialize; `null` stands in if one did not.
fn arg(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

macro_rules! calls {
    (@count) => { 0 };
    (@count $head:ident $($tail:ident)*) => { 1 + calls!(@count $($tail)*) };
    ($(
        $(#[$doc:meta])*
        $name:ident = $method:literal ($($field:ident: $ty:ty),*) -> $answer:ty;
    )*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name {
                $(pub $field: $ty,)*
            }

            impl CoreCall for $name {
                const METHOD: &'static str = $method;
                type Answer = $answer;
                fn args(self) -> Vec<Value> {
                    without_trailing_nulls(vec![$(arg(self.$field)),*])
                }
            }
        )*

        /// Every typed call's method and how many arguments it sends.
        pub const CALLS: &[(&str, usize)] = &[$(($method, calls!(@count $($field)*))),*];
    };
}

calls! {
    /// Every setting as stored: the core's preferences and the desktop's.
    DesktopSettings = "desktopSettings" () -> Settings;
    /// Store `settings`, edited from `base`: only what changed since `base` is written, so another
    /// window's changes stay. Answers the settings as stored.
    SaveDesktopSettings = "saveDesktopSettings" (settings: Settings, base: Settings) -> Settings;
    /// The user's own app themes, and a line per theme file that could not be used.
    CustomThemes = "customThemes" () -> CustomThemeReport;

    // Every other method a frontend calls; an answer in `Value` is read by the caller as before,
    // until its call names a contract type for it.
    /// A player's recent Lichess activity, by day.
    Activity = "activity" (username: Value) -> Value;
    AddAccount = "addAccount" (username: Value) -> Value;
    /// Sync a Chess.com player's games (by username; Chess.com needs no login).
    AddChesscomAccount = "addChesscomAccount" (username: Value) -> Value;
    /// Add players as friends without downloading their games.
    AddFriends = "addFriends" (usernames: Value) -> Value;
    /// Drills from a stored review's mistakes (one side's, when given); `added` counts new ones.
    AddMistakes = "addMistakes" (review_key: Value, color: Option<Value>) -> Value;
    /// Sign a ChatGPT account in through the browser.
    AiConnect = "aiConnect" () -> Value;
    /// Forget the ChatGPT sign-in on this profile.
    AiDisconnect = "aiDisconnect" () -> Value;
    /// Explain a move or puzzle from checked facts; refused during a live game.
    AiExplain = "aiExplain" (request: Value) -> Value;
    /// The models the connected account may use.
    AiModels = "aiModels" () -> Value;
    /// Choose the model (null: the account's first) and effort every AI feature uses.
    AiSetOptions = "aiSetOptions" (model: Value, effort: Value) -> Value;
    /// The ChatGPT connection, and the model and effort every AI feature uses.
    AiStatus = "aiStatus" () -> Value;
    AnswerMistake = "answerMistake" (id: Value, solved: Value) -> Value;
    BestMove = "bestMove" (moves: Value, level: Value, options: Option<Value>) -> Value;
    /// The games a connected account bookmarked on Lichess, from its side.
    BookmarkedGames = "bookmarkedGames" (account: Value) -> Value;
    /// The tournament a broadcast round is in: { tourId, roundId }.
    BroadcastRound = "broadcastRound" (id: Value) -> Value;
    Broadcasts = "broadcasts" (query: Option<Value>) -> Value;
    BroadcastTour = "broadcastTour" (id: Value) -> Value;
    /// Last profile data saved locally (no network); null fields when never fetched.
    CachedProfile = "cachedProfile" (username: Value) -> Value;
    /// Pending challenges to and from the connected accounts.
    Challenges = "challenges" () -> Value;
    /// The Chess.com players whose games are synced, with when each last synced.
    ChesscomAccounts = "chesscomAccounts" () -> Value;
    /// Delete the games and cached profile downloaded for one account, keeping the account.
    ClearAccountData = "clearAccountData" (username: Value) -> Value;
    ClearRepertoireMisses = "clearRepertoireMisses" (key: Value) -> Value;
    ClearRuns = "clearRuns" (kind: Option<Value>) -> Value;
    ClearVoiceHistory = "clearVoiceHistory" () -> Value;
    /// Closes the core: stops its services and releases the profile.
    Close = "close" () -> Value;
    /// Lichess's cached cloud evaluation of a position, or null when it has none.
    CloudEval = "cloudEval" (fen: Value, lines: Value) -> Value;
    /// Sign in through the browser. `username` is the account Lichess authorised.
    ConnectLichess = "connectLichess" (look: Option<Value>) -> Value;
    /// Creates an arena on Lichess run by the account.
    CreateTournament = "createTournament" (account: Value, arena: Value) -> Value;
    Crosstable = "crosstable" (a: Value, b: Value) -> Value;
    /// Delete the downloaded engine; the bundled and chosen ones are never touched.
    DeleteEngine = "deleteEngine" () -> Value;
    /// The engine that plays `variant` (standard chess when omitted): Stockfish, or for the\n   * variants it cannot play, the bundled Fairy-Stockfish.
    EngineStatus = "engineStatus" (variant: Option<Value>) -> Value;
    /// A finished Lichess game's PGN, for the analysis board.
    ExportGame = "exportGame" (id: Value) -> Value;
    ExportToLichessStudy = "exportToLichessStudy" (account: Value, study_id: Value, name: Value, pgn: Value) -> Value;
    /// Players the connected accounts follow on Lichess (needs the follow permission).
    Following = "following" () -> Value;
    GameLibraryOverview = "gameLibraryOverview" () -> Value;
    /// Filtered, bounded library rows; full PGN is fetched separately.
    GamePage = "gamePage" (query: Value) -> Value;
    /// Full PGN of one game; list rows omit it to keep the payload small.
    GamePgn = "gamePgn" (account: Value, id: Value) -> Value;
    GameRatingHistory = "gameRatingHistory" (account: Value) -> Value;
    /// Upload a PGN game to Lichess (as `account`, or anonymously when empty) for its page.
    ImportGame = "importGame" (account: Value, pgn: Value) -> Value;
    /// Patterns in an account's synced games, worked out on this computer.
    Insights = "insights" (query: Value) -> Value;
    /// Download and verify the latest official Stockfish build into KChess's own folder.\n   * `updated` is false when the installed copy was already the latest and nothing was downloaded.
    InstallEngine = "installEngine" () -> Value;
    /// Tournaments joined from KChess that have not ended.
    JoinedTournaments = "joinedTournaments" () -> Value;
    /// The top players of a rating category.
    Leaderboard = "leaderboard" (perf: Value) -> Value;
    /// The local library as saved: studies, played games, drills, sessions and training notes.
    Library = "library" () -> Value;
    /// Studies an account owns or belongs to on Lichess.
    LichessStudies = "lichessStudies" (account: Value) -> Value;
    /// One chapter of a public study ({ name, pgn }), its first when chapter is empty.
    LichessStudyChapter = "lichessStudyChapter" (id: Value, chapter: Value) -> Value;
    LichessStudyChapters = "lichessStudyChapters" (account: Value, id: Value) -> Value;
    LoadData = "loadData" () -> Value;
    /// Puzzles of rising difficulty, for Storm, Streak and Rush.
    LocalLadder = "localLadder" (query: Value) -> Value;
    Logout = "logout" (username: Value) -> Value;
    LogoutAll = "logoutAll" () -> Value;
    /// A master game's PGN from the masters database, for the analysis board.
    MastersGame = "mastersGame" (id: Value) -> Value;
    /// The reviewed games with positions in practice, most recently played first.
    MistakeGames = "mistakeGames" () -> Value;
    /// The mistake drills; the library's `mistakes` alone.
    Mistakes = "mistakes" () -> Value;
    /// Games the connected accounts are playing, most urgent first.
    OngoingGames = "ongoingGames" () -> Value;
    /// The Wikibooks description of a line of UCI moves from the starting position, or null when it has no page.
    OpeningDescription = "openingDescription" (moves: Value) -> Value;
    /// Month by month, the share of the Lichess games at the position before the line's last move that went on with it.
    OpeningPopularity = "openingPopularity" (moves: Value) -> Value;
    /// Send a move to a correspondence game that is not on the board (a reply from its\n   * notification): only while it is still this account's move at `fen`, and only once.
    PlayCorrespondence = "playCorrespondence" (account: Value, id: Value, fen: Value, r#move: Value) -> Value;
    PlayerPerf = "playerPerf" (username: Value, perf: Value) -> Value;
    PositionLookup = "positionLookup" (kind: Value, fen: Value, options: Option<Value>) -> Value;
    /// Online/playing/signal of the given users, for the game in progress.
    Presence = "presence" (usernames: Value) -> Value;
    Profile = "profile" (username: Value) -> Value;
    /// Many players' Lichess profiles at once (fewer details than `profile`), unknown names left out.
    Profiles = "profiles" (usernames: Value) -> Value;
    /// Read-only: the account's most recent puzzle attempts.
    PuzzleActivity = "puzzleActivity" (account: Value, max: Value) -> Value;
    /// One puzzle by its Lichess id; public, nothing is recorded.
    PuzzleById = "puzzleById" (id: Value) -> Value;
    /// The daily puzzle; public, nothing is recorded.
    PuzzleDaily = "puzzleDaily" () -> Value;
    /// Read-only: the account's puzzle dashboard.
    PuzzleDashboard = "puzzleDashboard" (account: Value, days: Value) -> Value;
    PuzzleDbCancel = "puzzleDbCancel" () -> Value;
    PuzzleDbDelete = "puzzleDbDelete" () -> Value;
    PuzzleDbInstall = "puzzleDbInstall" () -> Value;
    /// Local puzzle database (downloaded from database.lichess.org).
    PuzzleDbStatus = "puzzleDbStatus" () -> Value;
    PuzzleProgress = "puzzleProgress" (query: Value) -> Value;
    RatingHistory = "ratingHistory" (username: Value) -> Value;
    /// A player's latest public games, from their side; `rated` returns more of their rated ones.
    RecentGames = "recentGames" (username: Value, rated: Option<Value>) -> Value;
    /// Local training log (Practice, Downloaded and Rated attempts), kept in the puzzle database. It never leaves this computer.
    RecordPuzzleAttempt = "recordPuzzleAttempt" (attempt: Value) -> Value;
    RecordRepertoireMiss = "recordRepertoireMiss" (key: Value, fen: Value) -> Value;
    /// Whether a connected account follows or blocks a player on Lichess.
    Relation = "relation" (account: Value, username: Value) -> Value;
    RemoveAccount = "removeAccount" (username: Value) -> Value;
    RemoveArchivedGame = "removeArchivedGame" (id: Value) -> Value;
    /// Stop syncing a Chess.com player and delete their games.
    RemoveChesscomAccount = "removeChesscomAccount" (username: Value) -> Value;
    ResetUsage = "resetUsage" () -> Value;
    /// Put back a cleared repertoire's misses (Undo), added to any recorded since.
    RestoreRepertoireMisses = "restoreRepertoireMisses" (key: Value, misses: Value) -> Value;
    /// Reattach to a game in progress on any connected account.
    ResumeOnline = "resumeOnline" () -> Value;
    ReviewCancel = "reviewCancel" (key: Value) -> Value;
    /// The stored review of these moves from this position, if any; `variant` is a variant\n   * game's Lichess key.
    ReviewGet = "reviewGet" (fen: Value, moves: Value, variant: Option<Value>) -> Value;
    /// Answer a repertoire line right or wrong; its spaced-repetition schedule moves on. Returns every repertoire's schedule.
    ReviewRepertoireLine = "reviewRepertoireLine" (key: Value, line: Value, correct: Value) -> Value;
    /// Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess first).\n   * Returns what is stored so far; the rest arrives on `onReviewUpdate`.
    ReviewRequest = "reviewRequest" (request: Value) -> Value;
    ReviewStatus = "reviewStatus" () -> Value;
    /// Review summaries of up to one history page of games, by game id.
    ReviewSummaries = "reviewSummaries" (ids: Value) -> Value;
    RunSummary = "runSummary" (kind: Value) -> Value;
    /// Add or replace a played game; it becomes the most recent.
    SaveArchivedGame = "saveArchivedGame" (game: Value) -> Value;
    /// Local scores (Storm, Streak, Rush and the practice drills). They never leave this computer.
    SaveRun = "saveRun" (run: Value) -> Value;
    /// Keep an unfinished game or analysis so it can be resumed.
    SaveSession = "saveSession" (kind: Value, session: Value) -> Value;
    SaveSettings = "saveSettings" (settings: Value) -> Value;
    /// Local log of voice input (Settings › Voice). It never leaves this computer unless exported.
    SaveVoiceAttempt = "saveVoiceAttempt" (attempt: Value) -> Value;
    /// Players whose names start with `term` (three characters or more).
    SearchPlayers = "searchPlayers" (term: Value) -> Value;
    SendChat = "sendChat" (id: Value, room: Value, text: Value) -> Value;
    /// A Lichess private message from one of the connected accounts.
    SendMessage = "sendMessage" (account: Value, username: Value, text: Value) -> Value;
    /// The unfinished game or analysis of this kind kept to resume, or null; the library's `sessions` alone.
    Session = "session" (kind: Value) -> Value;
    /// Follow, unfollow, block or unblock a player on Lichess as a connected account.
    SetRelation = "setRelation" (account: Value, username: Value, action: Value) -> Value;
    /// The core settings as stored.
    StoredSettings = "settings" () -> Value;
    /// Remember a view preference for every KChess app on this profile; null forgets it.
    SetViewPreference = "setViewPreference" (key: Value, value: Value) -> Value;
    /// Start analysing a position (replacing any running analysis); updates arrive on `onAnalysis`.
    StartAnalysis = "startAnalysis" (request: Value) -> Value;
    /// Keep this connected account's Lichess event stream open while no game is being played, so\n   * challenges and tournament pairings arrive; empty closes it.
    StayConnected = "stayConnected" (account: Value) -> Value;
    /// Stop the running analysis; with `kill`, also closes the engine.
    StopAnalysis = "stopAnalysis" (kill: Option<Value>) -> Value;
    StopEngine = "stopEngine" () -> Value;
    /// Stop watching; with `session`, only that watch, never one begun after it.
    StopWatching = "stopWatching" (session: Option<Value>) -> Value;
    /// Read-only: anyone's public Storm dashboard.
    StormDashboard = "stormDashboard" (username: Value, days: Value) -> Value;
    StudyCommand = "studyCommand" (command: Value, incremental: Option<Value>, based_on: Option<Value>) -> Value;
    /// Download new games of one Chess.com player, or of every one.
    SyncChesscomGames = "syncChesscomGames" (username: Option<Value>) -> Value;
    SyncGames = "syncGames" (username: Option<Value>) -> Value;
    /// Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty).
    SyncLichessStudy = "syncLichessStudy" (request: Value) -> Value;
    /// What the players on the Following list did lately, newest first.
    Timeline = "timeline" () -> Value;
    /// One tournament; `page` picks the leaderboard page (10 players each, arenas only).
    Tournament = "tournament" (system: Value, id: Value, account: Value, page: Option<Value>) -> Value;
    /// Current arenas, and the Swiss events of the account's teams.
    Tournaments = "tournaments" (account: Value) -> Value;
    /// Trusts the Stockfish executable at `path`, chosen with the file picker.
    TrustEnginePath = "trustEnginePath" (path: Value) -> Value;
    /// Lichess TV channels and who is on each.
    TvChannels = "tvChannels" () -> Value;
    UpdateVoiceAttempt = "updateVoiceAttempt" (id: Value, update: Value) -> Value;
    /// Data downloaded from Lichess per account, and what is stored locally.
    Usage = "usage" () -> Value;
    /// How each page was last left (tabs, filters, toggles), by `kchess:` preference name.
    ViewPreferences = "viewPreferences" () -> Value;
    /// Base64 16-bit little-endian mono PCM at the started sample rate.
    VoiceAccept = "voiceAccept" (audio: Value) -> Value;
    /// Ends the utterance (push-to-talk released).
    VoiceFinish = "voiceFinish" () -> Value;
    /// Newest first.
    VoiceHistory = "voiceHistory" (limit: Value) -> Value;
    /// The voice input log as a document to export.
    VoiceHistoryDocument = "voiceHistoryDocument" () -> Value;
    /// Downloads, verifies and unpacks the speech model, reporting `voicemodel:progress`.
    VoiceModelInstall = "voiceModelInstall" () -> Value;
    /// Whether the offline speech model is installed, or being prepared.
    VoiceModelStatus = "voiceModelStatus" () -> Value;
    /// Offline speech recognition: listens for these phrases, discarding any utterance in progress.
    VoiceStart = "voiceStart" (grammar: Value, sample_rate: Value) -> Value;
    /// Releases the recognizer and its model.
    VoiceStop = "voiceStop" () -> Value;
    /// Watch a TV channel or any game by id (replacing what was watched); frames on `onWatch`.
    Watch = "watch" (target: Value) -> Value;
    /// Follow a broadcast round's live PGN; updates on `onBroadcast`.
    WatchBroadcast = "watchBroadcast" (round_id: Value) -> Value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> Settings {
        crate::defaults::default_settings().expect("the defaults decode")
    }

    #[test]
    fn each_call_sends_its_arguments_in_order() {
        assert!(DesktopSettings {}.args().is_empty());
        assert!(CustomThemes {}.args().is_empty());
        let mut base = settings();
        let edited = settings();
        base.frontend.sound_enabled = !edited.frontend.sound_enabled;
        let args = SaveDesktopSettings {
            settings: edited.clone(),
            base: base.clone(),
        }
        .args();
        assert_eq!(args, [arg(&edited), arg(&base)]);
        assert_ne!(args[0], args[1]);
        assert_eq!(
            CALLS[..3],
            [
                ("desktopSettings", 0),
                ("saveDesktopSettings", 2),
                ("customThemes", 0)
            ]
        );
        // A method has one typed call.
        let mut names: Vec<&str> = CALLS.iter().map(|(method, _)| *method).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CALLS.len());
    }

    #[test]
    fn an_optional_argument_left_out_is_not_sent() {
        assert!(StopAnalysis { kill: None }.args().is_empty());
        assert_eq!(
            StopAnalysis {
                kill: Some(json!(true))
            }
            .args(),
            [json!(true)]
        );
        let review = ReviewGet {
            fen: json!("8/8/8/8/8/8/8/K6k w - - 0 1"),
            moves: json!(["a1a2"]),
            variant: None,
        };
        assert_eq!(
            review.args(),
            [json!("8/8/8/8/8/8/8/K6k w - - 0 1"), json!(["a1a2"])]
        );
        // One left out between two given is sent as `null`, which the validator reads alike.
        let command = StudyCommand {
            command: json!({ "op": "rename" }),
            incremental: None,
            based_on: Some(json!("abc")),
        };
        assert_eq!(
            command.args(),
            [json!({ "op": "rename" }), Value::Null, json!("abc")]
        );
        assert_eq!(
            without_trailing_nulls(vec![Value::Null, json!(1), Value::Null, Value::Null]),
            [Value::Null, json!(1)]
        );
        assert!(CALLS.contains(&("reviewGet", 3)));
        assert!(CALLS.contains(&("stopAnalysis", 1)));
    }

    #[test]
    fn an_answer_is_read_in_the_call_s_shape() {
        let stored = settings();
        assert_eq!(decode::<DesktopSettings>(arg(&stored)), Ok(stored));
        let report = json!({ "themes": [], "dir": "/themes", "problems": ["bad.json: no name"] });
        let read = decode::<CustomThemes>(report).expect("a report");
        assert_eq!(read.dir, "/themes");
        assert_eq!(read.problems, ["bad.json: no name"]);
        let failed = decode::<SaveDesktopSettings>(json!(12)).expect_err("not settings");
        assert!(
            failed
                .message
                .starts_with("The core's answer to saveDesktopSettings could not be read: "),
            "{}",
            failed.message
        );
    }
}
