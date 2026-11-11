//! The core's methods: one declaration each, from which `core_api!` (`api.rs`) makes the typed
//! call a frontend sends (`calls`), the method table (`METHODS`), the argument check
//! (`check_arguments`), the trait the core implements (`CoreApi`, one part per area) and the
//! dispatch that runs a call by name on it (`dispatch`).
//!
//! Each argument's type is its check (`check::Validate`): the method receives the argument decoded,
//! checked and normalized; an optional one (after `;`) is `None` when absent or `null`.

use serde_json::Value;

use kchess_domain::misc::value::J;

pub use crate::api::{Assist, Method, Param, as_checked};
use crate::api::{answer, core_api, count_error};
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
        /// Every chapter of a study the account can read.
        LichessStudyChapters = "lichessStudyChapters" fn lichess_study_chapters(
            account: Username,
            id: LichessId
        ) -> Reply<Vec<crate::types::ChapterInput>>;
        /// One chapter of a public study, its first when chapter is empty.
        LichessStudyChapter = "lichessStudyChapter" fn lichess_study_chapter(
            id: LichessId,
            chapter: StudyId
        ) -> crate::types::ChapterInput;
        /// Upload a linked study's edits to Lichess; answers its chapters as Lichess holds them.
        SyncLichessStudy = "syncLichessStudy" fn sync_lichess_study(
            request: StudySyncRequest
        ) -> Reply<Vec<crate::types::ChapterInput>>;
        /// Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty).
        ExportToLichessStudy = "exportToLichessStudy" fn export_to_lichess_study(
            account: Username,
            study_id: StudyId,
            name: ChapterName,
            pgn: PgnText
        ) -> Reply<StudyExport>;
        /// A finished Lichess game's PGN, for the analysis board.
        ExportGame = "exportGame" fn export_game(id: GameId) -> String;
        /// A master game's PGN from the masters database, for the analysis board.
        #[assist(Lookups)]
        MastersGame = "mastersGame" fn masters_game(id: GameId) -> String;
        /// The Wikibooks description of a line of UCI moves from the starting position, or none
        /// when it has no page.
        #[assist(Lookups)]
        OpeningDescription = "openingDescription" fn opening_description(
            moves: Moves
        ) -> Option<crate::types::OpeningDescription>;
        /// Month by month, the share of the Lichess games at the position before the line's last
        /// move that went on with it.
        #[assist(Lookups)]
        OpeningPopularity = "openingPopularity" fn opening_popularity(
            moves: Moves
        ) -> Vec<PopularityMonth>;
        /// Lichess's cached cloud evaluation of a position, or none when it has none.
        #[assist(Analysis)]
        CloudEval = "cloudEval" fn cloud_eval(
            fen: Position,
            lines: Bounded<1, 5>
        ) -> Option<crate::types::CloudEval>;
    }

    /// The settings and the stored data.
    trait SettingsApi {
        LoadData = "loadData" fn load_data() -> AppData;
        /// The core settings as stored.
        StoredSettings = "settings" fn stored_settings() -> CoreSettings;
        /// Every setting as stored: the core's preferences and the desktop's.
        DesktopSettings = "desktopSettings" fn desktop_settings() -> Settings;
        SaveSettings = "saveSettings" fn save_settings(settings: CoreSettings) -> CoreSettings;
        /// Store `settings`, edited from `base`: only what changed since `base` is written, so
        /// another window's changes stay. Answers the settings as stored.
        #[writes(DesktopSettings)]
        SaveDesktopSettings = "saveDesktopSettings" fn save_desktop_settings(
            settings: Settings;
            base: Option<Settings>
        ) -> Settings;
    }

    /// Lichess accounts.
    trait AccountsApi {
        #[writes(Accounts)]
        AddAccount = "addAccount" fn add_account(username: Username) -> AppData;
        #[writes(Accounts)]
        Logout = "logout" fn logout(username: Username) -> AppData;
        #[writes(Accounts)]
        LogoutAll = "logoutAll" fn logout_all() -> AppData;
        #[writes(Accounts)]
        RemoveAccount = "removeAccount" fn remove_account(username: Username) -> AppData;
    }

    /// Chess.com players, whose games sync like Lichess games.
    trait ChesscomApi {
        /// The Chess.com players whose games are synced, with when each last synced.
        ChesscomAccounts = "chesscomAccounts" fn chesscom_accounts() -> Vec<ChesscomAccount>;
        /// Sync a Chess.com player's games (by username; Chess.com needs no login).
        #[writes(LichessGames, Accounts)]
        AddChesscomAccount = "addChesscomAccount" fn add_chesscom_account(
            username: ChesscomUsername
        ) -> Vec<ChesscomAccount>;
        /// Stop syncing a Chess.com player and delete their games.
        #[writes(LichessGames, Accounts)]
        RemoveChesscomAccount = "removeChesscomAccount" fn remove_chesscom_account(
            username: ChesscomUsername
        ) -> Vec<ChesscomAccount>;
        /// Download new games of one Chess.com player, or of every one.
        #[writes(LichessGames, Accounts)]
        SyncChesscomGames = "syncChesscomGames" fn sync_chesscom_games(;
            username: Option<ChesscomUsername>
        ) -> Vec<ChesscomAccount>;
    }

    /// Synced games, profiles and friends.
    trait GamesApi {
        #[writes(LichessGames, Accounts)]
        SyncGames = "syncGames" fn sync_games(; username: Option<Username>) -> AppData;
        /// Filtered, bounded library rows; full PGN is fetched separately.
        GamePage = "gamePage" fn game_page(
            query: crate::types::GamePageQuery
        ) -> crate::types::GamePage;
        GameLibraryOverview = "gameLibraryOverview" fn game_library_overview(
        ) -> crate::types::GameLibraryOverview;
        /// Patterns in an account's synced games, worked out on this computer.
        Insights = "insights" fn insights(query: InsightsQuery) -> InsightsReport;
        GameRatingHistory = "gameRatingHistory" fn game_rating_history(
            account: GameAccount
        ) -> Vec<RatingSeries>;
        /// Full PGN of one game; list rows omit it to keep the payload small.
        GamePgn = "gamePgn" fn game_pgn(account: GameAccount, id: GameId) -> Option<String>;
        /// Last profile data saved locally (no network); null fields when never fetched.
        CachedProfile = "cachedProfile" fn cached_profile(
            username: Username
        ) -> crate::types::CachedProfile;
        /// A Lichess user as Lichess sent it.
        Profile = "profile" fn profile(username: Username) -> Value;
        /// A player's rating history as Lichess sent it.
        RatingHistory = "ratingHistory" fn rating_history(username: Username) -> Value;
        /// Sign in through the browser. `username` is the account Lichess authorised.
        #[writes(Accounts)]
        ConnectLichess = "connectLichess" fn connect_lichess(;
            look: Option<Value>
        ) -> ConnectedLichess;
        /// Keep this connected account's Lichess event stream open while no game is being
        /// played, so challenges and tournament pairings arrive; empty closes it.
        StayConnected = "stayConnected" fn stay_connected(account: OptionalAccount) -> ();
        /// Delete the games and cached profile downloaded for one account, keeping the account.
        #[writes(LichessGames, Accounts, Usage)]
        ClearAccountData = "clearAccountData" fn clear_account_data(
            username: Username
        ) -> AppData;
        /// Players the connected accounts follow on Lichess (needs the follow permission).
        Following = "following" fn following() -> FollowingReport;
        /// Add players as friends without downloading their games.
        #[writes(Accounts)]
        AddFriends = "addFriends" fn add_friends(usernames: FriendList) -> AppData;
    }

    /// ChatGPT explanations.
    trait AiApi {
        /// The ChatGPT connection, and the model and effort every AI feature uses.
        AiStatus = "aiStatus" fn ai_status() -> AiConnection;
        /// Sign a ChatGPT account in through the browser.
        AiConnect = "aiConnect" fn ai_connect() -> AiConnection;
        /// Forget the ChatGPT sign-in on this profile.
        AiDisconnect = "aiDisconnect" fn ai_disconnect() -> AiConnection;
        /// The models the connected account may use.
        AiModels = "aiModels" fn ai_models() -> AiModelList;
        /// Choose the model (null: the account's first) and effort every AI feature uses.
        AiSetOptions = "aiSetOptions" fn ai_set_options(
            model: Option<AiModelName>,
            effort: AiEffort
        ) -> AiConnection;
        /// Explain a move or puzzle from checked facts; refused during a live game.
        #[assist(Explain)]
        AiExplain = "aiExplain" fn ai_explain(request: AiRequest) -> AiExplanation;
    }

    /// The computer's engine, analysis and game review.
    trait EngineApi {
        /// The engine that plays `variant` (standard chess when omitted): Stockfish, or for the
        /// variants it cannot play, the bundled Fairy-Stockfish.
        EngineStatus = "engineStatus" fn engine_status(;
            variant: Option<crate::Variant>
        ) -> crate::types::EngineStatus;
        /// Download and verify the latest official Stockfish build into KChess's own folder.
        /// `updated` is false when the installed copy was already the latest and nothing was
        /// downloaded.
        #[writes(Engine)]
        InstallEngine = "installEngine" fn install_engine() -> EngineInstalled;
        /// Delete the downloaded engine; the bundled and chosen ones are never touched.
        #[writes(Engine)]
        DeleteEngine = "deleteEngine" fn delete_engine() -> ();
        StopEngine = "stopEngine" fn stop_engine() -> ();
        #[assist(Engine)]
        BestMove = "bestMove" fn best_move(
            moves: Moves,
            level: EngineLevel;
            options: Option<BestMoveOptions>
        ) -> String;
        /// Start analysing a position (replacing any running analysis); updates arrive on
        /// `engine:analysis`.
        #[assist(Analysis)]
        StartAnalysis = "startAnalysis" fn start_analysis(
            request: AnalysisRequest
        ) -> u64;
        /// Stop the running analysis; with `kill`, also closes the engine.
        StopAnalysis = "stopAnalysis" fn stop_analysis(;
            kill: Option<bool>
        ) -> ();
        /// The stored review of these moves from this position, if any; `variant` is a variant
        /// game's Lichess key.
        #[assist(StoredReview)]
        #[check(crate::check::review_get)]
        ReviewGet = "reviewGet" fn review_get(
            fen: Fen,
            moves: Moves;
            variant: Option<String>
        ) -> Option<StoredReview>;
        /// Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess
        /// first). Returns what is stored so far; the rest arrives on `review:update`.
        #[assist(Review)]
        ReviewRequest = "reviewRequest" fn review_request(
            request: crate::types::ReviewRequest
        ) -> Option<StoredReview>;
        ReviewCancel = "reviewCancel" fn review_cancel(key: ReviewKey) -> ();
        ReviewStatus = "reviewStatus" fn review_status() -> crate::types::ReviewStatus;
        /// Review summaries of up to one history page of games, by game id.
        ReviewSummaries = "reviewSummaries" fn review_summaries(ids: GameIds) -> Keyed<ReviewSummary>;
        /// Trusts the Stockfish executable at `path`, chosen with the file picker.
        TrustEnginePath = "trustEnginePath" fn trust_engine_path(path: String) -> ();
    }

    /// Online play and challenges.
    trait OnlineApi {
        StartOnline = "startOnline" fn start_online(
            options: crate::types::OnlineOptions
        ) -> StartResult;
        /// Reattach to a game in progress on any connected account.
        ResumeOnline = "resumeOnline" fn resume_online() -> Option<Resumed>;
        CancelOnline = "cancelOnline" fn cancel_online() -> ();
        PlayOnline = "playOnline" fn play_online(id: GameId, r#move: Uci) -> ();
        /// Send a move to a correspondence game that is not on the board (a reply from its
        /// notification): only while it is still this account's move at `fen`, and only once.
        PlayCorrespondence = "playCorrespondence" fn play_correspondence(
            account: Username,
            id: GameId,
            fen: FullFen,
            r#move: Uci
        ) -> ();
        OnlineAction = "onlineAction" fn online_action(
            id: GameId,
            action: crate::types::OnlineAction
        ) -> ();
        /// The private player chat of the game being played (needs the setting on).
        OnlineChat = "onlineChat" fn online_chat(id: GameId) -> Vec<ChatLine>;
        SendChat = "sendChat" fn send_chat(id: GameId, room: ChatRoom, text: ChatText) -> ();
        /// Online/playing/signal of the given users, for the game in progress.
        Presence = "presence" fn presence(usernames: PresenceUsers) -> PresenceReport;
        /// Pending challenges to and from the connected accounts.
        Challenges = "challenges" fn challenges() -> Vec<ChallengeInfo>;
        AcceptChallenge = "acceptChallenge" fn accept_challenge(id: GameId) -> ();
        DeclineChallenge = "declineChallenge" fn decline_challenge(
            id: GameId,
            reason: DeclineReason
        ) -> ();
        /// Withdraw a challenge you sent.
        CancelChallenge = "cancelChallenge" fn cancel_challenge(id: GameId) -> ();
        /// Games the connected accounts are playing, most urgent first.
        OngoingGames = "ongoingGames" fn ongoing_games() -> Vec<OngoingGame>;
        /// Open one of those games on the online board (a correspondence game, or a live one).
        OpenGame = "openGame" fn open_game(account: Username, id: GameId) -> ();
    }

    /// Lichess players: profiles, ratings, messages and relations.
    trait SocialApi {
        PlayerPerf = "playerPerf" fn player_perf(username: Username, perf: PerfType) -> PerfStats;
        Crosstable = "crosstable" fn crosstable(
            a: Username,
            b: Username
        ) -> crate::types::Crosstable;
        /// A player's latest public games, from their side; `rated` returns more of their rated
        /// ones.
        RecentGames = "recentGames" fn recent_games(
            username: Username;
            rated: Option<bool>
        ) -> Vec<LichessGame>;
        /// A Lichess private message from one of the connected accounts.
        SendMessage = "sendMessage" fn send_message(
            account: Username,
            username: Username,
            text: MessageText
        ) -> Reply<MessageSent>;
        /// Whether a connected account follows or blocks a player on Lichess.
        Relation = "relation" fn relation(
            account: Username,
            username: Username
        ) -> Reply<LichessRelation>;
        /// Follow, unfollow, block or unblock a player on Lichess as a connected account.
        /// Answers `true` once done.
        SetRelation = "setRelation" fn set_relation(
            account: Username,
            username: Username,
            action: RelationAction
        ) -> Reply<bool>;
        /// Many players' Lichess profiles at once (fewer details than `profile`), unknown names
        /// left out, as Lichess sent them.
        Profiles = "profiles" fn profiles(usernames: FriendList) -> Vec<Value>;
        /// The top players of a rating category.
        Leaderboard = "leaderboard" fn leaderboard(perf: PerfType) -> Vec<LeaderboardEntry>;
        /// Players whose names start with `term` (three characters or more).
        SearchPlayers = "searchPlayers" fn search_players(
            term: Username
        ) -> Vec<PlayerSuggestion>;
        /// A player's recent Lichess activity, by day.
        Activity = "activity" fn activity(username: Username) -> Vec<ActivityDay>;
        /// What the players on the Following list did lately, newest first.
        Timeline = "timeline" fn timeline() -> Vec<TimelineEntry>;
        /// Upload a PGN game to Lichess (as `account`, or anonymously when empty) for its page.
        ImportGame = "importGame" fn import_game(
            account: OptionalAccount,
            pgn: PgnText
        ) -> ImportedGame;
        /// The games a connected account bookmarked on Lichess, from its side.
        BookmarkedGames = "bookmarkedGames" fn bookmarked_games(
            account: Username
        ) -> Reply<Vec<LichessGame>>;
    }

    /// Lichess TV and broadcasts.
    trait SpectateApi {
        /// Lichess TV channels and who is on each.
        TvChannels = "tvChannels" fn tv_channels() -> Vec<TvChannel>;
        /// Watch a TV channel or any game by id (replacing what was watched); frames on
        /// `watch:frame`. Answers the watch's session.
        Watch = "watch" fn watch(target: WatchTarget) -> u64;
        /// Follow a broadcast round's live PGN; updates on `watch:broadcast`. Answers the watch's
        /// session.
        WatchBroadcast = "watchBroadcast" fn watch_broadcast(round_id: LichessId) -> u64;
        /// Stop watching; with `session`, only that watch, never one begun after it.
        StopWatching = "stopWatching" fn stop_watching(; session: Option<WatchSession>) -> ();
        Broadcasts = "broadcasts" fn broadcasts(
            ; query: Option<BroadcastQuery>
        ) -> Vec<BroadcastSummary>;
        BroadcastTour = "broadcastTour" fn broadcast_tour(id: LichessId) -> BroadcastTourDetail;
        /// The tournament a broadcast round is in.
        BroadcastRound = "broadcastRound" fn broadcast_round(id: LichessId) -> BroadcastRoundPlace;
    }

    /// Lichess tournaments.
    trait TournamentsApi {
        /// Current arenas, and the Swiss events of the account's teams.
        Tournaments = "tournaments" fn tournaments(account: OptionalAccount) -> TournamentList;
        /// One tournament; `page` picks the leaderboard page (10 players each, arenas only).
        Tournament = "tournament" fn tournament(
            system: TournamentSystem,
            id: TournamentId,
            account: OptionalAccount;
            page: Option<StandingPage>
        ) -> TournamentDetail;
        /// Answers `true` once joined.
        #[writes(Tournaments)]
        JoinTournament = "joinTournament" fn join_tournament(
            system: TournamentSystem,
            id: TournamentId,
            account: Username;
            password: Option<TournamentPassword>
        ) -> Reply<bool>;
        /// Answers `true` once left.
        #[writes(Tournaments)]
        LeaveTournament = "leaveTournament" fn leave_tournament(
            system: TournamentSystem,
            id: TournamentId,
            account: Username
        ) -> Reply<bool>;
        /// Creates an arena on Lichess run by the account.
        #[writes(Tournaments)]
        CreateTournament = "createTournament" fn create_tournament(
            account: Username,
            arena: NewArena
        ) -> Reply<TournamentSummary>;
    }

    /// Puzzles, the puzzle database, training runs and the voice input log.
    trait PuzzlesApi {
        /// Lichess puzzle training. Rated results change the account's Lichess puzzle rating.
        PuzzleNext = "puzzleNext" fn puzzle_next(
            request: crate::types::PuzzleRequest
        ) -> Reply<PuzzleDraw>;
        /// Report a puzzle result to Lichess (needs `puzzle:write`).
        #[writes(Puzzles)]
        PuzzleSolve = "puzzleSolve" fn puzzle_solve(
            request: PuzzleSolveRequest
        ) -> Reply<PuzzleSolveResult>;
        /// The daily puzzle; public, nothing is recorded.
        PuzzleDaily = "puzzleDaily" fn puzzle_daily() -> Puzzle;
        /// One puzzle by its Lichess id; public, nothing is recorded.
        PuzzleById = "puzzleById" fn puzzle_by_id(id: PuzzleId) -> Puzzle;
        /// Read-only: the account's puzzle dashboard, as Lichess reports it.
        PuzzleDashboard = "puzzleDashboard" fn puzzle_dashboard(
            account: Username,
            days: Days
        ) -> Reply<Value>;
        /// Read-only: the account's most recent puzzle attempts.
        PuzzleActivity = "puzzleActivity" fn puzzle_activity(
            account: Username,
            max: ActivityMax
        ) -> Reply<Vec<PuzzleActivityEntry>>;
        /// Read-only: anyone's public Storm dashboard, as Lichess reports it.
        StormDashboard = "stormDashboard" fn storm_dashboard(
            username: Username,
            days: Days
        ) -> Value;
        /// Local puzzle database (downloaded from database.lichess.org).
        PuzzleDbStatus = "puzzleDbStatus" fn puzzle_db_status() -> crate::types::PuzzleDbStatus;
        #[writes(Puzzles)]
        PuzzleDbInstall = "puzzleDbInstall" fn puzzle_db_install() -> crate::types::PuzzleDbStatus;
        PuzzleDbCancel = "puzzleDbCancel" fn puzzle_db_cancel() -> ();
        #[writes(Puzzles)]
        PuzzleDbDelete = "puzzleDbDelete" fn puzzle_db_delete() -> crate::types::PuzzleDbStatus;
        LocalPuzzles = "localPuzzles" fn local_puzzles(query: LocalPuzzleQuery) -> Vec<Puzzle>;
        /// Puzzles of rising difficulty, for Storm, Streak and Rush.
        LocalLadder = "localLadder" fn local_ladder(query: LocalLadderQuery) -> Vec<Puzzle>;
        /// Local training log (Practice, Downloaded and Rated attempts), kept in the puzzle
        /// database. It never leaves this computer.
        #[writes(Puzzles)]
        RecordPuzzleAttempt = "recordPuzzleAttempt" fn record_puzzle_attempt(
            attempt: PuzzleAttemptInput
        ) -> crate::types::PuzzleProgress;
        PuzzleProgress = "puzzleProgress" fn puzzle_progress(
            query: PuzzleProgressQuery
        ) -> crate::types::PuzzleProgress;
        /// Local scores (Storm, Streak, Rush and the practice drills). They never leave this
        /// computer.
        #[writes(Puzzles)]
        SaveRun = "saveRun" fn save_run(run: RunInput) -> RunSaved;
        RunSummary = "runSummary" fn run_summary(kind: RunKind) -> crate::types::RunSummary;
        #[writes(Puzzles)]
        ClearRuns = "clearRuns" fn clear_runs(; kind: Option<RunKind>) -> ();
        /// Local log of voice input (Settings › Voice). It never leaves this computer unless
        /// exported. Answers the new entry's id.
        #[writes(VoiceHistory)]
        SaveVoiceAttempt = "saveVoiceAttempt" fn save_voice_attempt(
            attempt: VoiceAttemptInput
        ) -> i64;
        #[writes(VoiceHistory)]
        UpdateVoiceAttempt = "updateVoiceAttempt" fn update_voice_attempt(
            id: VoiceId,
            update: VoiceUpdate
        ) -> ();
        /// Newest first.
        VoiceHistory = "voiceHistory" fn voice_history(limit: VoiceLimit) -> Vec<VoiceAttempt>;
        #[writes(VoiceHistory)]
        ClearVoiceHistory = "clearVoiceHistory" fn clear_voice_history() -> ();
        /// The voice input log as a document to export.
        VoiceHistoryDocument = "voiceHistoryDocument" fn voice_history_document() -> String;
    }

    /// Offline speech recognition and its model.
    trait VoiceApi {
        /// Offline speech recognition: listens for these phrases, discarding any utterance in
        /// progress.
        VoiceStart = "voiceStart" fn voice_start(
            grammar: VoiceGrammar,
            sample_rate: SampleRate
        ) -> ();
        /// Base64 16-bit little-endian mono PCM at the started sample rate.
        VoiceAccept = "voiceAccept" fn voice_accept(audio: VoiceAudio) -> crate::voice::Heard;
        /// Ends the utterance (push-to-talk released).
        VoiceFinish = "voiceFinish" fn voice_finish() -> crate::voice::Heard;
        /// Releases the recognizer and its model.
        VoiceStop = "voiceStop" fn voice_stop() -> ();
        /// Whether the offline speech model is installed, or being prepared.
        VoiceModelStatus = "voiceModelStatus" fn voice_model_status() -> crate::voice::Status;
        /// Downloads, verifies and unpacks the speech model, reporting `voicemodel:progress`.
        VoiceModelInstall = "voiceModelInstall" fn voice_model_install() -> ();
    }

    /// The local library: studies, played games, drills, sessions and repertoire notes.
    trait LibraryApi {
        /// The local library as saved: studies, played games, drills, sessions and training
        /// notes.
        Library = "library" fn library() -> crate::types::Library;
        /// One change to the studies. `incremental` answers only the studies it changed;
        /// `based_on` is the chapter's PGN as the window last read it, so a save over a change
        /// made elsewhere is refused.
        #[writes(Studies)]
        StudyCommand = "studyCommand" fn study_command(
            command: crate::types::StudyCommand;
            incremental: Option<bool>,
            based_on: Option<PgnText>
        ) -> StudyCommandResult;
        /// Add or replace a played game; it becomes the most recent.
        #[writes(PlayedGames)]
        SaveArchivedGame = "saveArchivedGame" fn save_archived_game(game: ArchivedGame) -> ();
        #[writes(PlayedGames)]
        RemoveArchivedGame = "removeArchivedGame" fn remove_archived_game(id: LibraryId) -> ();
        /// Drills from a stored review's mistakes (one side's, when given); `added` counts new
        /// ones.
        #[writes(Drills)]
        AddMistakes = "addMistakes" fn add_mistakes(
            review_key: ReviewKey;
            color: Option<Side>
        ) -> MistakesAdded;
        /// Schedule a drill again; answers every drill.
        #[writes(Drills)]
        AnswerMistake = "answerMistake" fn answer_mistake(id: LibraryId, solved: bool) -> Vec<Value>;
        /// The reviewed games with positions in practice, most recently played first.
        MistakeGames = "mistakeGames" fn mistake_games() -> Vec<MistakeGame>;
        /// Keep an unfinished game or analysis so it can be resumed.
        #[writes(Sessions)]
        SaveSession = "saveSession" fn save_session(
            kind: SessionKind,
            session: SessionObject
        ) -> ();
        /// Tournaments joined from KChess that have not ended.
        JoinedTournaments = "joinedTournaments" fn joined_tournaments() -> Vec<Value>;
        /// The unfinished game or analysis of this kind kept to resume, or null; the library's
        /// `sessions` alone.
        Session = "session" fn session(kind: SessionKind) -> Option<Value>;
        /// The mistake drills; the library's `mistakes` alone.
        Mistakes = "mistakes" fn mistakes() -> Vec<Value>;
        /// Answers every repertoire's missed positions.
        #[writes(Repertoire)]
        RecordRepertoireMiss = "recordRepertoireMiss" fn record_repertoire_miss(
            key: RepertoireKey,
            fen: Position
        ) -> Value;
        #[writes(Repertoire)]
        ClearRepertoireMisses = "clearRepertoireMisses" fn clear_repertoire_misses(
            key: RepertoireKey
        ) -> Value;
        /// Put back a cleared repertoire's misses (Undo), added to any recorded since.
        #[writes(Repertoire)]
        RestoreRepertoireMisses = "restoreRepertoireMisses" fn restore_repertoire_misses(
            key: RepertoireKey,
            misses: MissCounts
        ) -> Value;
        /// Answer a repertoire line right or wrong; its spaced-repetition schedule moves on.
        /// Returns every repertoire's schedule.
        #[writes(Repertoire)]
        ReviewRepertoireLine = "reviewRepertoireLine" fn review_repertoire_line(
            key: RepertoireKey,
            line: RepertoireLine,
            correct: bool
        ) -> Value;
        /// How each page was last left (tabs, filters, toggles), by `kchess:` preference name.
        ViewPreferences = "viewPreferences" fn view_preferences() -> serde_json::Map<String, Value>;
        /// Remember a view preference for every KChess app on this profile; null forgets it.
        #[writes(ViewPreferences)]
        SetViewPreference = "setViewPreference" fn set_view_preference(
            key: ViewPreferenceKey;
            value: Option<Value>
        ) -> ();
    }

    /// Network usage and the user's themes.
    trait SystemApi {
        /// Data downloaded from Lichess per account, and what is stored locally.
        Usage = "usage" fn usage() -> UsageReport;
        #[writes(Usage)]
        ResetUsage = "resetUsage" fn reset_usage() -> ();
        /// The user's own app themes from the profile's themes folder, with a line per file that
        /// could not be used.
        CustomThemes = "customThemes" fn custom_themes() -> CustomThemeReport;
    }
}
