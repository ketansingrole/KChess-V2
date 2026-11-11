//! The core's methods: one declaration each, from which `core_api!` (`api.rs`) makes the typed
//! call a frontend sends (`calls`), the method table (`METHODS`), the argument check
//! (`check_arguments`), the trait the core implements (`CoreApi`, one part per area) and the
//! dispatch that runs a call by name on it (`dispatch`).
//!
//! Each argument names its check (`kchess_domain::misc::checks`): the method receives what the
//! check normalized, `Raw(…)` the argument as sent once it passed, and `Optional(…)` checks only
//! an argument that was given (`null` otherwise).

use serde_json::Value;

use kchess_domain::misc::checks as c;
use kchess_domain::misc::value::J;

pub use crate::api::{Assist, Method, Param, as_checked};
use crate::api::{Optional, Raw, answer, core_api, count_error, run_check, take};
use crate::check::*;
use crate::error::CoreError;
use crate::types::*;

/// The method `name`, as `METHODS` declares it.
pub fn method(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|method| method.name == name)
}

core_api! {
    /// Lichess studies, positions and the opening explorer.
    trait StudiesApi {
        #[assist(Lookups)]
        #[check(crate::check::position_lookup)]
        PositionLookup = "positionLookup" fn position_lookup(
            kind: PositionLookupKind,
            fen: Fen;
            options: Option<LookupOptions>
        ) -> crate::types::PositionLookup;
        /// Studies an account owns or belongs to on Lichess.
        LichessStudies = "lichessStudies" fn lichess_studies(
            account: Username
        ) -> Reply<Vec<LichessStudy>>;
        LichessStudyChapters = "lichessStudyChapters" fn lichess_study_chapters(
            account: Value = Raw(c::username),
            id: Value = Raw(c::lichess_id)
        ) -> Value;
        /// One chapter of a public study ({ name, pgn }), its first when chapter is empty.
        LichessStudyChapter = "lichessStudyChapter" fn lichess_study_chapter(
            id: Value = c::lichess_id,
            chapter: Value = c::study_id
        ) -> Value;
        /// Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty).
        SyncLichessStudy = "syncLichessStudy" fn sync_lichess_study(
            request: Value = c::study_sync_request
        ) -> Value;
        ExportToLichessStudy = "exportToLichessStudy" fn export_to_lichess_study(
            account: Value = Raw(c::username),
            study_id: Value = Raw(c::study_id),
            name: Value = Raw(c::chapter_name),
            pgn: Value = Raw(c::pgn_text)
        ) -> Value;
        /// A finished Lichess game's PGN, for the analysis board.
        ExportGame = "exportGame" fn export_game(id: Value = Raw(c::game_id)) -> Value;
        /// A master game's PGN from the masters database, for the analysis board.
        #[assist(Lookups)]
        MastersGame = "mastersGame" fn masters_game(id: Value = Raw(c::game_id)) -> Value;
        /// The Wikibooks description of a line of UCI moves from the starting position, or null
        /// when it has no page.
        #[assist(Lookups)]
        OpeningDescription = "openingDescription" fn opening_description(
            moves: Value = Raw(c::moves)
        ) -> Value;
        /// Month by month, the share of the Lichess games at the position before the line's last
        /// move that went on with it.
        #[assist(Lookups)]
        OpeningPopularity = "openingPopularity" fn opening_popularity(
            moves: Value = Raw(c::moves)
        ) -> Value;
        /// Lichess's cached cloud evaluation of a position, or null when it has none.
        #[assist(Analysis)]
        CloudEval = "cloudEval" fn cloud_eval(
            fen: Value = Raw(c::position),
            lines: Value = Raw(c::lines)
        ) -> Value;
    }

    /// The settings and the stored data.
    trait SettingsApi {
        LoadData = "loadData" fn load_data() -> Value;
        /// The core settings as stored.
        StoredSettings = "settings" fn stored_settings() -> Value;
        /// Every setting as stored: the core's preferences and the desktop's.
        DesktopSettings = "desktopSettings" fn desktop_settings() -> Settings;
        SaveSettings = "saveSettings" fn save_settings(settings: Value = c::core_settings) -> Value;
        /// Store `settings`, edited from `base`: only what changed since `base` is written, so
        /// another window's changes stay. Answers the settings as stored.
        #[writes(DesktopSettings)]
        SaveDesktopSettings = "saveDesktopSettings" fn save_desktop_settings(
            settings: Settings = c::settings;
            base: Option<Settings> = Optional(c::settings)
        ) -> Settings;
    }

    /// Lichess accounts.
    trait AccountsApi {
        #[writes(Accounts)]
        AddAccount = "addAccount" fn add_account(username: Value = c::username) -> Value;
        #[writes(Accounts)]
        Logout = "logout" fn logout(username: Value = c::username) -> Value;
        #[writes(Accounts)]
        LogoutAll = "logoutAll" fn logout_all() -> Value;
        #[writes(Accounts)]
        RemoveAccount = "removeAccount" fn remove_account(username: Value = c::username) -> Value;
    }

    /// Chess.com players, whose games sync like Lichess games.
    trait ChesscomApi {
        /// The Chess.com players whose games are synced, with when each last synced.
        ChesscomAccounts = "chesscomAccounts" fn chesscom_accounts() -> Value;
        /// Sync a Chess.com player's games (by username; Chess.com needs no login).
        #[writes(LichessGames, Accounts)]
        AddChesscomAccount = "addChesscomAccount" fn add_chesscom_account(
            username: Value = c::chesscom_username
        ) -> Value;
        /// Stop syncing a Chess.com player and delete their games.
        #[writes(LichessGames, Accounts)]
        RemoveChesscomAccount = "removeChesscomAccount" fn remove_chesscom_account(
            username: Value = c::chesscom_username
        ) -> Value;
        /// Download new games of one Chess.com player, or of every one.
        #[writes(LichessGames, Accounts)]
        SyncChesscomGames = "syncChesscomGames" fn sync_chesscom_games(;
            username: Option<Value> = Optional(c::chesscom_username)
        ) -> Value;
    }

    /// Synced games, profiles and friends.
    trait GamesApi {
        #[writes(LichessGames, Accounts)]
        SyncGames = "syncGames" fn sync_games(;
            username: Option<Value> = Optional(c::username)
        ) -> Value;
        /// Filtered, bounded library rows; full PGN is fetched separately.
        GamePage = "gamePage" fn game_page(query: Value = c::game_page_query) -> Value;
        GameLibraryOverview = "gameLibraryOverview" fn game_library_overview() -> Value;
        /// Patterns in an account's synced games, worked out on this computer.
        Insights = "insights" fn insights(query: Value = c::insights_query) -> Value;
        GameRatingHistory = "gameRatingHistory" fn game_rating_history(
            account: Value = c::game_account
        ) -> Value;
        /// Full PGN of one game; list rows omit it to keep the payload small.
        GamePgn = "gamePgn" fn game_pgn(
            account: Value = c::game_account,
            id: Value = c::game_id
        ) -> Value;
        /// Last profile data saved locally (no network); null fields when never fetched.
        CachedProfile = "cachedProfile" fn cached_profile(username: Value = c::username) -> Value;
        Profile = "profile" fn profile(username: Value = c::username) -> Value;
        RatingHistory = "ratingHistory" fn rating_history(username: Value = c::username) -> Value;
        /// Sign in through the browser. `username` is the account Lichess authorised.
        #[writes(Accounts)]
        ConnectLichess = "connectLichess" fn connect_lichess(;
            look: Option<Value> = c::any
        ) -> Value;
        /// Keep this connected account's Lichess event stream open while no game is being
        /// played, so challenges and tournament pairings arrive; empty closes it.
        StayConnected = "stayConnected" fn stay_connected(
            account: Value = c::optional_account
        ) -> Value;
        /// Delete the games and cached profile downloaded for one account, keeping the account.
        #[writes(LichessGames, Accounts, Usage)]
        ClearAccountData = "clearAccountData" fn clear_account_data(
            username: Value = c::username
        ) -> Value;
        /// Players the connected accounts follow on Lichess (needs the follow permission).
        Following = "following" fn following() -> Value;
        /// Add players as friends without downloading their games.
        #[writes(Accounts)]
        AddFriends = "addFriends" fn add_friends(usernames: Value = c::friend_list) -> Value;
    }

    /// ChatGPT explanations.
    trait AiApi {
        /// The ChatGPT connection, and the model and effort every AI feature uses.
        AiStatus = "aiStatus" fn ai_status() -> Value;
        /// Sign a ChatGPT account in through the browser.
        AiConnect = "aiConnect" fn ai_connect() -> Value;
        /// Forget the ChatGPT sign-in on this profile.
        AiDisconnect = "aiDisconnect" fn ai_disconnect() -> Value;
        /// The models the connected account may use.
        AiModels = "aiModels" fn ai_models() -> Value;
        /// Choose the model (null: the account's first) and effort every AI feature uses.
        AiSetOptions = "aiSetOptions" fn ai_set_options(
            model: Value = c::ai_model,
            effort: Value = c::ai_effort
        ) -> Value;
        /// Explain a move or puzzle from checked facts; refused during a live game.
        #[assist(Explain)]
        AiExplain = "aiExplain" fn ai_explain(request: Value = c::ai_request) -> Value;
    }

    /// The computer's engine, analysis and game review.
    trait EngineApi {
        /// The engine that plays `variant` (standard chess when omitted): Stockfish, or for the
        /// variants it cannot play, the bundled Fairy-Stockfish.
        EngineStatus = "engineStatus" fn engine_status(;
            variant: Option<Value> = Optional(Raw(c::variant))
        ) -> Value;
        /// Download and verify the latest official Stockfish build into KChess's own folder.
        /// `updated` is false when the installed copy was already the latest and nothing was
        /// downloaded.
        #[writes(Engine)]
        InstallEngine = "installEngine" fn install_engine() -> Value;
        /// Delete the downloaded engine; the bundled and chosen ones are never touched.
        #[writes(Engine)]
        DeleteEngine = "deleteEngine" fn delete_engine() -> Value;
        StopEngine = "stopEngine" fn stop_engine() -> Value;
        #[assist(Engine)]
        BestMove = "bestMove" fn best_move(
            moves: Value = c::moves,
            level: Value = c::level;
            options: Option<Value> = c::best_move_options
        ) -> Value;
        /// Start analysing a position (replacing any running analysis); updates arrive on
        /// `engine:analysis`.
        #[assist(Analysis)]
        StartAnalysis = "startAnalysis" fn start_analysis(
            request: Value = Raw(c::analysis_request)
        ) -> Value;
        /// Stop the running analysis; with `kill`, also closes the engine.
        StopAnalysis = "stopAnalysis" fn stop_analysis(;
            kill: Option<Value> = Optional(c::boolean)
        ) -> Value;
        /// The stored review of these moves from this position, if any; `variant` is a variant
        /// game's Lichess key.
        #[assist(StoredReview)]
        ReviewGet = "reviewGet" fn review_get(
            fen: Value = c::review_fen,
            moves: Value = c::review_moves;
            variant: Option<Value> = c::review_variant
        ) -> Value;
        /// Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess
        /// first). Returns what is stored so far; the rest arrives on `review:update`.
        #[assist(Review)]
        ReviewRequest = "reviewRequest" fn review_request(
            request: Value = c::review_request
        ) -> Value;
        ReviewCancel = "reviewCancel" fn review_cancel(key: Value = c::review_key) -> Value;
        ReviewStatus = "reviewStatus" fn review_status() -> Value;
        /// Review summaries of up to one history page of games, by game id.
        ReviewSummaries = "reviewSummaries" fn review_summaries(ids: Value = c::game_ids) -> Value;
        /// Trusts the Stockfish executable at `path`, chosen with the file picker.
        TrustEnginePath = "trustEnginePath" fn trust_engine_path(path: Value = c::any) -> Value;
    }

    /// Online play, challenges and Lichess players.
    trait OnlineApi {
        StartOnline = "startOnline" fn start_online(options: Value = c::online_options) -> Value;
        /// Reattach to a game in progress on any connected account.
        ResumeOnline = "resumeOnline" fn resume_online() -> Value;
        CancelOnline = "cancelOnline" fn cancel_online() -> Value;
        PlayOnline = "playOnline" fn play_online(
            id: Value = c::game_id,
            r#move: Value = c::uci
        ) -> Value;
        /// Send a move to a correspondence game that is not on the board (a reply from its
        /// notification): only while it is still this account's move at `fen`, and only once.
        PlayCorrespondence = "playCorrespondence" fn play_correspondence(
            account: Value = c::username,
            id: Value = c::game_id,
            fen: Value = c::fen,
            r#move: Value = c::uci
        ) -> Value;
        OnlineAction = "onlineAction" fn online_action(
            id: Value = c::game_id,
            action: Value = c::action
        ) -> Value;
        /// The private player chat of the game being played (needs the setting on).
        OnlineChat = "onlineChat" fn online_chat(id: Value = c::game_id) -> Value;
        SendChat = "sendChat" fn send_chat(
            id: Value = c::game_id,
            room: Value = c::chat_room,
            text: Value = c::chat_text
        ) -> Value;
        /// Online/playing/signal of the given users, for the game in progress.
        Presence = "presence" fn presence(usernames: Value = c::usernames) -> Value;
        /// Pending challenges to and from the connected accounts.
        Challenges = "challenges" fn challenges() -> Value;
        AcceptChallenge = "acceptChallenge" fn accept_challenge(id: Value = c::game_id) -> Value;
        DeclineChallenge = "declineChallenge" fn decline_challenge(
            id: Value = c::game_id,
            reason: Value = c::decline_reason
        ) -> Value;
        /// Withdraw a challenge you sent.
        CancelChallenge = "cancelChallenge" fn cancel_challenge(id: Value = c::game_id) -> Value;
        /// Games the connected accounts are playing, most urgent first.
        OngoingGames = "ongoingGames" fn ongoing_games() -> Value;
        /// Open one of those games on the online board (a correspondence game, or a live one).
        OpenGame = "openGame" fn open_game(
            account: Value = c::username,
            id: Value = c::game_id
        ) -> Value;
        PlayerPerf = "playerPerf" fn player_perf(
            username: Value = c::username,
            perf: Value = c::perf_type
        ) -> Value;
        Crosstable = "crosstable" fn crosstable(a: Value = c::username, b: Value = c::username) -> Value;
        /// A player's latest public games, from their side; `rated` returns more of their rated
        /// ones.
        RecentGames = "recentGames" fn recent_games(
            username: Value = c::username;
            rated: Option<Value> = Optional(c::boolean)
        ) -> Value;
        /// A Lichess private message from one of the connected accounts.
        SendMessage = "sendMessage" fn send_message(
            account: Value = c::username,
            username: Value = c::username,
            text: Value = c::message_text
        ) -> Value;
        /// Whether a connected account follows or blocks a player on Lichess.
        Relation = "relation" fn relation(
            account: Value = c::username,
            username: Value = c::username
        ) -> Value;
        /// Follow, unfollow, block or unblock a player on Lichess as a connected account.
        SetRelation = "setRelation" fn set_relation(
            account: Value = c::username,
            username: Value = c::username,
            action: Value = c::relation_action
        ) -> Value;
        /// Many players' Lichess profiles at once (fewer details than `profile`), unknown names
        /// left out.
        Profiles = "profiles" fn profiles(usernames: Value = c::friend_list) -> Value;
        /// The top players of a rating category.
        Leaderboard = "leaderboard" fn leaderboard(perf: Value = c::perf_type) -> Value;
        /// Players whose names start with `term` (three characters or more).
        SearchPlayers = "searchPlayers" fn search_players(term: Value = c::username) -> Value;
        /// A player's recent Lichess activity, by day.
        Activity = "activity" fn activity(username: Value = c::username) -> Value;
        /// What the players on the Following list did lately, newest first.
        Timeline = "timeline" fn timeline() -> Value;
        /// Upload a PGN game to Lichess (as `account`, or anonymously when empty) for its page.
        ImportGame = "importGame" fn import_game(
            account: Value = c::optional_account,
            pgn: Value = c::pgn_text
        ) -> Value;
        /// The games a connected account bookmarked on Lichess, from its side.
        BookmarkedGames = "bookmarkedGames" fn bookmarked_games(account: Value = c::username) -> Value;
    }

    /// Lichess TV and broadcasts.
    trait SpectateApi {
        /// Lichess TV channels and who is on each.
        TvChannels = "tvChannels" fn tv_channels() -> Value;
        /// Watch a TV channel or any game by id (replacing what was watched); frames on
        /// `watch:frame`.
        Watch = "watch" fn watch(target: Value = c::watch_target) -> Value;
        /// Follow a broadcast round's live PGN; updates on `watch:broadcast`.
        WatchBroadcast = "watchBroadcast" fn watch_broadcast(round_id: Value = c::lichess_id) -> Value;
        /// Stop watching; with `session`, only that watch, never one begun after it.
        StopWatching = "stopWatching" fn stop_watching(;
            session: Option<Value> = Optional(c::watch_session)
        ) -> Value;
        Broadcasts = "broadcasts" fn broadcasts(; query: Option<Value> = c::broadcast_query) -> Value;
        BroadcastTour = "broadcastTour" fn broadcast_tour(id: Value = c::lichess_id) -> Value;
        /// The tournament a broadcast round is in: { tourId, roundId }.
        BroadcastRound = "broadcastRound" fn broadcast_round(id: Value = c::lichess_id) -> Value;
    }

    /// Lichess tournaments.
    trait TournamentsApi {
        /// Current arenas, and the Swiss events of the account's teams.
        Tournaments = "tournaments" fn tournaments(account: Value = c::optional_account) -> Value;
        /// One tournament; `page` picks the leaderboard page (10 players each, arenas only).
        Tournament = "tournament" fn tournament(
            system: Value = c::tournament_system,
            id: Value = c::tournament_id,
            account: Value = c::optional_account;
            page: Option<Value> = Raw(c::optional_page)
        ) -> Value;
        #[writes(Tournaments)]
        JoinTournament = "joinTournament" fn join_tournament(
            system: Value = c::tournament_system,
            id: Value = c::tournament_id,
            account: Value = c::username;
            password: Option<Value> = c::tournament_password
        ) -> Value;
        #[writes(Tournaments)]
        LeaveTournament = "leaveTournament" fn leave_tournament(
            system: Value = c::tournament_system,
            id: Value = c::tournament_id,
            account: Value = c::username
        ) -> Value;
        /// Creates an arena on Lichess run by the account.
        #[writes(Tournaments)]
        CreateTournament = "createTournament" fn create_tournament(
            account: Value = c::username,
            arena: Value = c::new_arena
        ) -> Value;
    }

    /// Puzzles, the puzzle database, training runs and the voice input log.
    trait PuzzlesApi {
        /// Lichess puzzle training. Rated results change the account's Lichess puzzle rating.
        PuzzleNext = "puzzleNext" fn puzzle_next(request: Value = c::puzzle_request) -> Value;
        /// Report a puzzle result to Lichess (needs `puzzle:write`).
        #[writes(Puzzles)]
        PuzzleSolve = "puzzleSolve" fn puzzle_solve(request: Value = c::puzzle_solve) -> Value;
        /// The daily puzzle; public, nothing is recorded.
        PuzzleDaily = "puzzleDaily" fn puzzle_daily() -> Value;
        /// One puzzle by its Lichess id; public, nothing is recorded.
        PuzzleById = "puzzleById" fn puzzle_by_id(id: Value = c::puzzle_id) -> Value;
        /// Read-only: the account's puzzle dashboard.
        PuzzleDashboard = "puzzleDashboard" fn puzzle_dashboard(
            account: Value = c::username,
            days: Value = c::days
        ) -> Value;
        /// Read-only: the account's most recent puzzle attempts.
        PuzzleActivity = "puzzleActivity" fn puzzle_activity(
            account: Value = c::username,
            max: Value = c::activity_max
        ) -> Value;
        /// Read-only: anyone's public Storm dashboard.
        StormDashboard = "stormDashboard" fn storm_dashboard(
            username: Value = c::username,
            days: Value = c::days
        ) -> Value;
        /// Local puzzle database (downloaded from database.lichess.org).
        PuzzleDbStatus = "puzzleDbStatus" fn puzzle_db_status() -> Value;
        #[writes(Puzzles)]
        PuzzleDbInstall = "puzzleDbInstall" fn puzzle_db_install() -> Value;
        PuzzleDbCancel = "puzzleDbCancel" fn puzzle_db_cancel() -> Value;
        #[writes(Puzzles)]
        PuzzleDbDelete = "puzzleDbDelete" fn puzzle_db_delete() -> Value;
        LocalPuzzles = "localPuzzles" fn local_puzzles(query: Value = c::local_query) -> Value;
        /// Puzzles of rising difficulty, for Storm, Streak and Rush.
        LocalLadder = "localLadder" fn local_ladder(query: Value = c::ladder_query) -> Value;
        /// Local training log (Practice, Downloaded and Rated attempts), kept in the puzzle
        /// database. It never leaves this computer.
        #[writes(Puzzles)]
        RecordPuzzleAttempt = "recordPuzzleAttempt" fn record_puzzle_attempt(
            attempt: Value = c::puzzle_attempt
        ) -> Value;
        PuzzleProgress = "puzzleProgress" fn puzzle_progress(
            query: Value = c::puzzle_progress_query
        ) -> Value;
        /// Local scores (Storm, Streak, Rush and the practice drills). They never leave this
        /// computer.
        #[writes(Puzzles)]
        SaveRun = "saveRun" fn save_run(run: Value = c::run_input) -> Value;
        RunSummary = "runSummary" fn run_summary(kind: Value = c::run_kind) -> Value;
        #[writes(Puzzles)]
        ClearRuns = "clearRuns" fn clear_runs(; kind: Option<Value> = Optional(c::run_kind)) -> Value;
        /// Local log of voice input (Settings › Voice). It never leaves this computer unless
        /// exported.
        #[writes(VoiceHistory)]
        SaveVoiceAttempt = "saveVoiceAttempt" fn save_voice_attempt(
            attempt: Value = c::voice_attempt
        ) -> Value;
        #[writes(VoiceHistory)]
        UpdateVoiceAttempt = "updateVoiceAttempt" fn update_voice_attempt(
            id: Value = c::voice_id,
            update: Value = c::voice_update
        ) -> Value;
        /// Newest first.
        VoiceHistory = "voiceHistory" fn voice_history(limit: Value = c::voice_limit) -> Value;
        #[writes(VoiceHistory)]
        ClearVoiceHistory = "clearVoiceHistory" fn clear_voice_history() -> Value;
        /// The voice input log as a document to export.
        VoiceHistoryDocument = "voiceHistoryDocument" fn voice_history_document() -> Value;
    }

    /// Offline speech recognition and its model.
    trait VoiceApi {
        /// Offline speech recognition: listens for these phrases, discarding any utterance in
        /// progress.
        VoiceStart = "voiceStart" fn voice_start(
            grammar: Value = Raw(c::voice_grammar),
            sample_rate: Value = Raw(c::sample_rate)
        ) -> Value;
        /// Base64 16-bit little-endian mono PCM at the started sample rate.
        VoiceAccept = "voiceAccept" fn voice_accept(audio: Value = Raw(c::voice_audio)) -> Value;
        /// Ends the utterance (push-to-talk released).
        VoiceFinish = "voiceFinish" fn voice_finish() -> Value;
        /// Releases the recognizer and its model.
        VoiceStop = "voiceStop" fn voice_stop() -> Value;
        /// Whether the offline speech model is installed, or being prepared.
        VoiceModelStatus = "voiceModelStatus" fn voice_model_status() -> Value;
        /// Downloads, verifies and unpacks the speech model, reporting `voicemodel:progress`.
        VoiceModelInstall = "voiceModelInstall" fn voice_model_install() -> Value;
    }

    /// The local library: studies, played games, drills, sessions and repertoire notes.
    trait LibraryApi {
        /// The local library as saved: studies, played games, drills, sessions and training
        /// notes.
        Library = "library" fn library() -> Value;
        #[writes(Studies)]
        StudyCommand = "studyCommand" fn study_command(
            command: Value = c::study_command_shape;
            incremental: Option<Value> = Optional(Raw(c::boolean)),
            based_on: Option<Value> = Optional(Raw(c::pgn_text))
        ) -> Value;
        /// Add or replace a played game; it becomes the most recent.
        #[writes(PlayedGames)]
        SaveArchivedGame = "saveArchivedGame" fn save_archived_game(
            game: Value = c::archived_game_shape
        ) -> Value;
        #[writes(PlayedGames)]
        RemoveArchivedGame = "removeArchivedGame" fn remove_archived_game(id: LibraryId) -> ();
        /// Drills from a stored review's mistakes (one side's, when given); `added` counts new
        /// ones.
        #[writes(Drills)]
        AddMistakes = "addMistakes" fn add_mistakes(
            review_key: Value = c::review_key;
            color: Option<Value> = Optional(c::side)
        ) -> Value;
        #[writes(Drills)]
        AnswerMistake = "answerMistake" fn answer_mistake(
            id: Value = c::library_id,
            solved: Value = c::boolean
        ) -> Value;
        /// The reviewed games with positions in practice, most recently played first.
        MistakeGames = "mistakeGames" fn mistake_games() -> Value;
        /// Keep an unfinished game or analysis so it can be resumed.
        #[writes(Sessions)]
        SaveSession = "saveSession" fn save_session(
            kind: Value = c::session_kind,
            session: Value = Raw(c::session_object)
        ) -> Value;
        /// Tournaments joined from KChess that have not ended.
        JoinedTournaments = "joinedTournaments" fn joined_tournaments() -> Value;
        /// The unfinished game or analysis of this kind kept to resume, or null; the library's
        /// `sessions` alone.
        Session = "session" fn session(kind: Value = c::session_kind) -> Value;
        /// The mistake drills; the library's `mistakes` alone.
        Mistakes = "mistakes" fn mistakes() -> Value;
        #[writes(Repertoire)]
        RecordRepertoireMiss = "recordRepertoireMiss" fn record_repertoire_miss(
            key: Value = c::repertoire_key,
            fen: Value = c::position
        ) -> Value;
        #[writes(Repertoire)]
        ClearRepertoireMisses = "clearRepertoireMisses" fn clear_repertoire_misses(
            key: Value = c::repertoire_key
        ) -> Value;
        /// Put back a cleared repertoire's misses (Undo), added to any recorded since.
        #[writes(Repertoire)]
        RestoreRepertoireMisses = "restoreRepertoireMisses" fn restore_repertoire_misses(
            key: Value = c::repertoire_key,
            misses: Value = Raw(c::session_object)
        ) -> Value;
        /// Answer a repertoire line right or wrong; its spaced-repetition schedule moves on.
        /// Returns every repertoire's schedule.
        #[writes(Repertoire)]
        ReviewRepertoireLine = "reviewRepertoireLine" fn review_repertoire_line(
            key: Value = c::repertoire_key,
            line: Value = c::repertoire_line,
            correct: Value = c::boolean
        ) -> Value;
        /// How each page was last left (tabs, filters, toggles), by `kchess:` preference name.
        ViewPreferences = "viewPreferences" fn view_preferences() -> Value;
        /// Remember a view preference for every KChess app on this profile; null forgets it.
        #[writes(ViewPreferences)]
        SetViewPreference = "setViewPreference" fn set_view_preference(
            key: Value = c::view_preference_key,
            value: Value = c::any
        ) -> Value;
    }

    /// Network usage and the user's themes.
    trait SystemApi {
        /// Data downloaded from Lichess per account, and what is stored locally.
        Usage = "usage" fn usage() -> Value;
        #[writes(Usage)]
        ResetUsage = "resetUsage" fn reset_usage() -> Value;
        /// The user's own app themes from the profile's themes folder, with a line per file that
        /// could not be used.
        CustomThemes = "customThemes" fn custom_themes() -> CustomThemeReport;
    }
}
