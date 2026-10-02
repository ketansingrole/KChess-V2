import type { components, paths } from '@lichess-org/types'

export const APPEARANCES = ['system', 'light', 'dark'] as const
export type Appearance = (typeof APPEARANCES)[number]
export const COORDINATE_MODES = ['none', 'inside', 'outside'] as const
export type CoordinateMode = (typeof COORDINATE_MODES)[number]
export const ENGINE_LEVELS = ['low', 'medium', 'high'] as const
export type EngineLevel = (typeof ENGINE_LEVELS)[number]
export const PROMOTION_MODES = ['ask', 'queen', 'premove'] as const
/** How a pawn reaching the last rank is promoted: always ask, always to a queen, or a queen only when premoving. */
export type PromotionMode = (typeof PROMOTION_MODES)[number]
export const PIECE_ANIMATIONS = ['none', 'fast', 'normal', 'slow'] as const
/** How long a piece takes to slide to its square. */
export type PieceAnimation = (typeof PIECE_ANIMATIONS)[number]
export const NOTIFICATION_KINDS = [
  'opponentMove',
  'lowTime',
  'gameEvents',
  'computerMove',
  'test',
] as const
/** What a desktop notification is about; each kind (except `test`) has its own switch in Settings. */
export type NotificationKind = (typeof NOTIFICATION_KINDS)[number]
export const ONLINE_ACTIONS = ['resign', 'abort', 'takeback', 'declineTakeback'] as const
export type OnlineAction = (typeof ONLINE_ACTIONS)[number]
export const CHALLENGE_COLORS = ['random', 'white', 'black'] as const
export type ChallengeColor = (typeof CHALLENGE_COLORS)[number]
export const PUZZLE_DIFFICULTIES = ['easiest', 'easier', 'normal', 'harder', 'hardest'] as const
/** How far from your Lichess puzzle rating the next puzzle is aimed. */
export type PuzzleDifficulty = (typeof PUZZLE_DIFFICULTIES)[number]
/** Everything the runs table (local scores) records. */
export const RUN_KINDS = [
  'storm',
  'streak',
  'rush',
  'coordinates',
  'squareColor',
  'knightPath',
  'endgame',
] as const
export type RunKind = (typeof RUN_KINDS)[number]
/** Where a spoken phrase was said. */
export const VOICE_SOURCES = ['computer', 'coordinates'] as const
export type VoiceSource = (typeof VOICE_SOURCES)[number]
/**
 * What became of a spoken phrase. Moves: `played` at once, `pending` a choice that then was
 * `confirmed` (by voice), `picked` (clicked), `cancelled`, `replaced` by another phrase or
 * `abandoned` (the position moved on). `invalid` matched no legal move, `unclear` was too uncertain
 * to use, `command` was “confirm”/“cancel”. Coordinates: `correct` or `wrong`.
 */
export const VOICE_OUTCOMES = [
  'played',
  'pending',
  'confirmed',
  'picked',
  'cancelled',
  'replaced',
  'abandoned',
  'invalid',
  'unclear',
  'command',
  'correct',
  'wrong',
] as const
export type VoiceOutcome = (typeof VOICE_OUTCOMES)[number]

/** Colors of one light or dark variant of an app theme, as `#rgb` / `#rrggbb`. Optional ones are derived from the rest. */
export interface ThemePalette {
  /** Page background. */
  bg: string
  /** Main text. */
  text: string
  /** Accent: buttons, links, focus. */
  primary: string
  /** Slightly raised areas. */
  muted?: string
  /** Cards. */
  elevated?: string
  /** Hover and selected backgrounds. */
  accented?: string
  border?: string
  textMuted?: string
  success?: string
  warning?: string
  error?: string
  info?: string
}

/** A color theme for the whole app. A theme with only one variant uses it in both light and dark mode. */
export interface AppTheme {
  id: string
  name: string
  light?: ThemePalette
  dark?: ThemePalette
  /** Loaded from the user's themes folder rather than shipped with the app. */
  custom?: boolean
}

export interface CustomThemeReport {
  themes: AppTheme[]
  /** The folder custom theme files are read from. */
  dir: string
  /** One line per file that could not be used. */
  problems: string[]
}

export interface Settings {
  appearance: Appearance
  boardTheme: string
  /** Color theme of the app itself in light mode: `kchess` (the default look), a preset, or a custom theme id. */
  lightTheme: string
  /** The same for dark mode; the two are chosen independently. */
  darkTheme: string
  /** One of the piece sets shipped in `app/assets/pieces`. */
  pieceSet: string
  pieceAnimation: PieceAnimation
  coordinates: CoordinateMode
  soundEnabled: boolean
  soundVolume: number
  enginePath: string
  /** Let the player queue a move while it is the opponent's turn. */
  premove: boolean
  promotion: PromotionMode
  /** Dot the squares a selected piece can move to. */
  showLegalMoves: boolean
  /** Master switch for desktop notifications. */
  notificationsEnabled: boolean
  /** Alert (in the app) while KChess is the window in use. */
  notifyActive: boolean
  /** Alert (a system notification) while KChess is in the background: unfocused, minimized or hidden. */
  notifyBackground: boolean
  /** Your online opponent has moved. */
  notifyOpponentMove: boolean
  /** Your clock is running low in an online game. */
  notifyLowTime: boolean
  /** An online game starts or ends. */
  notifyGameEvents: boolean
  /** Stockfish has replied in a computer game. */
  notifyComputerMove: boolean
  /** Let the operating system play its notification sound (KChess's own game sounds are separate). */
  notifySound: boolean
  /** Voice input listens only while Space or the on-screen button is held. */
  voicePushToTalk: boolean
  /** A spoken move waits for "confirm" (or a click) before it is played. */
  voiceConfirmMoves: boolean
  /** Keep a local log of what voice input heard and what came of it, to find what it mishears. */
  voiceHistory: boolean
  /** Check stable releases on startup and periodically while KChess is open. */
  updateAutoCheck: boolean
  /** Download verified updates in the background when a check finds one. */
  updateAutoDownload: boolean
  /** Apply a downloaded update when the user quits; never restart during play. */
  updateInstallOnQuit: boolean
}

export interface AppUpdateStatus {
  phase:
    | 'idle'
    | 'checking'
    | 'up-to-date'
    | 'available'
    | 'downloading'
    | 'downloaded'
    | 'error'
    | 'disabled'
  currentVersion: string
  canCheck: boolean
  canInstall: boolean
  /** Why this installation needs a manual download, or cannot check at all. */
  reason?: string
  version?: string
  releaseDate?: string
  checkedAt?: number
  progress?: { percent: number; transferred: number; total: number; bytesPerSecond: number }
  error?: string
}

/** The operating system's microphone permission for KChess (`granted` where the OS has none). */
export type MicrophoneStatus = 'granted' | 'denied' | 'restricted' | 'not-determined' | 'unknown'
export interface MicrophoneAccess {
  status: MicrophoneStatus
  /** A privacy settings page can be opened (macOS and Windows). */
  canOpenSettings: boolean
  /** A macOS dev build: the permission belongs to the terminal or editor that launched KChess. */
  launchedFromTerminal: boolean
}

export interface NotificationRequest {
  kind: NotificationKind
  title: string
  body: string
}

/** Why a notification was not shown, when it was not. */
export type NotificationSkip =
  'disabled' | 'category-off' | 'window-state' | 'unsupported' | 'failed'
export interface NotificationResult {
  shown: boolean
  /** `system` is an operating-system notification; `in-app` is an alert inside the window. */
  via?: 'system' | 'in-app'
  skipped?: NotificationSkip
  /** The operating system's reason, when it refused the notification (`skipped` is `failed`). */
  error?: string
}

export interface LichessAccount {
  username: string
  connected: boolean
  lastSyncedAt?: number
}

export interface LichessGame {
  id: string
  account: string
  createdAt: number
  lastMoveAt: number
  rated: boolean
  speed: string
  perf: string
  status: string
  winner?: 'white' | 'black'
  color: 'white' | 'black'
  opponent: string
  opponentRating?: number
  playerRating?: number
  ratingDiff?: number
  opening?: string
  moves: string
  pgn?: string
}

export interface AppData {
  settings: Settings
  accounts: LichessAccount[]
  games: LichessGame[]
}

/** Types published by the Lichess team in `@lichess-org/types`. */
export type LichessUser = components['schemas']['UserExtended']
export type LichessRatingHistory = components['schemas']['RatingHistory']
export type LichessNowPlaying =
  paths['/api/account/playing']['get']['responses'][200]['content']['application/json']
export type LichessGameFullEvent = components['schemas']['GameFullEvent']
export type LichessGameStateEvent = components['schemas']['GameStateEvent']
export type LichessGameStartEvent = components['schemas']['GameStartEvent']
export type LichessGameFinishEvent = components['schemas']['GameFinishEvent']
export type LichessChallengeEvent = components['schemas']['ChallengeEvent']
export type LichessChallengeCanceledEvent = components['schemas']['ChallengeCanceledEvent']
export type LichessChallengeDeclinedEvent = components['schemas']['ChallengeDeclinedEvent']
export type LichessChatLineEvent = components['schemas']['ChatLineEvent']
export type LichessOpponentGoneEvent = components['schemas']['OpponentGoneEvent']
export type OnlineEvent =
  | LichessGameFullEvent
  | LichessGameStartEvent
  | LichessGameFinishEvent
  | LichessGameStateEvent
  | LichessChallengeEvent
  | LichessChallengeCanceledEvent
  | LichessChallengeDeclinedEvent
  | LichessChatLineEvent
  | LichessOpponentGoneEvent

export interface EngineStatus {
  ready: boolean
  /** The executable in use (a downloaded or chosen one), or empty when the bundled engine is. */
  path: string
  bundled: boolean
  /** The Stockfish KChess downloaded, whether or not it is the one in use. */
  managed: { installed: boolean; path: string; version?: string }
  /** Downloading is only wired up for macOS releases. */
  canDownload: boolean
}

export interface OnlineOptions {
  minutes: number
  increment: number
  color: ChallengeColor
  /** Rated games change the Lichess rating; casual ones do not. */
  rated: boolean
  target?: string
  /** Which connected account plays; defaults to the first connected one. */
  account?: string
}

/** A player one of the connected accounts follows on Lichess. */
export interface FollowedUser {
  username: string
  title?: string
  ratings: { bullet?: number; blitz?: number; rapid?: number; classical?: number }
  /** Connected accounts that follow them. */
  followedBy: string[]
  /** Already a friend in KChess (or one of your own accounts). */
  alreadyAdded: boolean
  /** Removed as a friend earlier; not suggested again unless asked. */
  dismissed: boolean
}

export interface FollowingProblem {
  account: string
  message: string
  /** Connecting the account again grants the missing permission. */
  needsReconnect: boolean
}

export interface FollowingReport {
  users: FollowedUser[]
  problems: FollowingProblem[]
}

/** What a request to Lichess was for; usage is broken down by it. */
export type UsageKind = 'games' | 'profile' | 'play' | 'presence' | 'puzzles' | 'database' | 'other'

export interface UsageCell {
  requests: number
  /** Bytes received, after decompression. */
  bytesIn: number
}

export interface AccountUsage {
  total: UsageCell
  byKind: Partial<Record<UsageKind, UsageCell>>
}

export interface AccountStorage {
  games: number
  /** Approximate size of the stored games. */
  bytes: number
  /** Profile and rating data cached for the account. */
  cacheBytes: number
}

export interface UsageReport {
  /** When counting started; undefined when nothing has been counted. */
  since?: number
  /** Lower-cased username → downloads from Lichess; the empty key is traffic with no account (login, etc.). */
  accounts: Record<string, AccountUsage>
  /** Lower-cased username → what is kept on this computer. */
  storage: Record<string, AccountStorage>
  /** Size of the database file. */
  dbBytes: number
}

/** Lichess's view of one user's connection. `signal` runs 1 (poor, lag > 500 ms) to 4 (great, lag < 150 ms). */
export interface UserPresence {
  online: boolean
  playing: boolean
  signal?: number
}

export interface PresenceReport {
  /** Lower-cased username → status; users Lichess did not report are absent. */
  users: Record<string, UserPresence>
  /** Round trip of the status request itself, i.e. this computer's latency to Lichess. */
  latencyMs: number
}

/** One puzzle, whichever source it came from. */
export interface Puzzle {
  id: string
  /** The position the player starts from: it is the player's move, after the opponent's `lastMove`. */
  fen: string
  /** UCI of the opponent's move that led here (for the board highlight). */
  lastMove?: string
  /** UCI moves, alternating player, opponent, … and ending with the player's move. */
  solution: string[]
  rating: number
  themes: string[]
  plays?: number
  /** Lichess game the position came from. */
  gameId?: string
}

export interface PuzzleGlicko {
  rating?: number
  deviation?: number
  provisional?: boolean
}

export interface PuzzleRequest {
  /** Connected account to train as; empty trains anonymously (no rating, nothing recorded). */
  account: string
  /** A puzzle theme key (`mateIn2`, `fork`, …) or `mix`. */
  angle: string
  difficulty: PuzzleDifficulty
  color?: 'white' | 'black'
}

export interface PuzzleDraw {
  puzzle: Puzzle
  /** The account's current puzzle rating, when Lichess reports it. */
  glicko?: PuzzleGlicko
}

export interface PuzzleSolveRequest {
  account: string
  angle: string
  id: string
  win: boolean
  /** Rated results change the Lichess puzzle rating; unrated ones are only marked as seen. */
  rated: boolean
}

export interface PuzzleSolveResult {
  /** Rating change Lichess applied; absent for unrated results. */
  ratingDiff?: number
}

/** Returned instead of data when the account's Lichess login lacks (or lost) the puzzle permission. */
export interface NeedsReconnect {
  needsReconnect: true
}

export interface PuzzlePerformance {
  firstWins: number
  nb: number
  performance: number
  puzzleRatingAvg: number
  replayWins: number
}

export interface PuzzleDashboard {
  days: number
  global: PuzzlePerformance
  themes: Record<string, { theme: string; results: PuzzlePerformance }>
}

export interface PuzzleActivityEntry {
  /** When it was played, in ms since the epoch. */
  date: number
  win: boolean
  puzzle: Puzzle
}

export interface StormDashboard {
  high: { allTime: number; day: number; month: number; week: number }
  days: {
    /** `YYYY/M/D` (Lichess's own key). */
    _id: string
    combo: number
    errors: number
    highest: number
    moves: number
    runs: number
    score: number
    time: number
  }[]
}

/** A puzzle-database download or import in progress (or finished), streamed to the window. */
export interface PuzzleDbProgress {
  phase: 'downloading' | 'importing' | 'done' | 'cancelled' | 'failed'
  /** Compressed bytes received so far. */
  received: number
  /** Total compressed bytes, when the server said. */
  total?: number
  /** Puzzles kept so far. */
  kept: number
  message?: string
}

export interface PuzzleDbStatus {
  installed: boolean
  count: number
  /** Approximate size on disk. */
  bytes: number
  importedAt?: number
  /** True while a download runs; its progress arrives as events. */
  busy: boolean
}

export interface LocalPuzzleQuery {
  theme?: string
  minRating?: number
  maxRating?: number
  count: number
}

export interface LocalLadderQuery {
  from: number
  to: number
  count: number
}

export interface RunInput {
  kind: RunKind
  /** Which flavour of the kind (`3min`, `find`, `kq-vs-k`, …). */
  variant: string
  score: number
  /** Extra numbers and words shown with the score (accuracy, combo, …). */
  detail: Record<string, string | number | boolean>
}

export interface RunRecord extends RunInput {
  id: number
  playedAt: number
}

export interface RunSummary {
  kind: RunKind
  /** Best score per variant. */
  best: Record<string, { score: number; playedAt: number }>
  /** Newest first. */
  recent: RunRecord[]
  /** All runs of this kind. */
  total: number
}

export interface RunSaved {
  summary: RunSummary
  /** The run beat every earlier run of its variant. */
  isBest: boolean
}

/** One recognized word and the recognizer's confidence in it (0–1). */
export interface VoiceWord {
  word: string
  conf: number
}

export interface VoiceModelProgress {
  phase: 'checking' | 'downloading' | 'preparing'
  received: number
  total?: number
}

export interface VoiceAttemptInput {
  source: VoiceSource
  /** The recognizer's text, as heard. */
  heard: string
  /** Average word confidence, 0–1. */
  confidence: number
  words: VoiceWord[]
  outcome: VoiceOutcome
  /** How KChess read it: SAN choices joined by `|`, a square, or `confirm`/`cancel`. */
  parsed?: string
  /** What the player meant, when known: the square asked for, or the move they went on to play. */
  expected?: string
  /** The position it was said in (FEN), so the phrase can be parsed again later. */
  fen?: string
  /** An earlier attempt at the same move or question that this one repeats. */
  retryOf?: number
}

export interface VoiceAttempt extends VoiceAttemptInput {
  id: number
  at: number
}

/** What is learned about an attempt after it was logged. */
export interface VoiceAttemptUpdate {
  outcome?: VoiceOutcome
  expected?: string
}

export interface BestMoveOptions {
  /** Start from this position instead of the standard one. */
  fen?: string
  /** Think for this many milliseconds instead of the level's default. */
  movetime?: number
}

export interface DesktopApi {
  /** Save redacted runtime diagnostics to a location chosen in the native dialog. */
  exportDiagnostics(): Promise<boolean>
  /** Frameless-chrome window controls (Windows/Linux); no-ops where the OS draws its own chrome. */
  windowMinimize(): Promise<void>
  windowToggleMaximize(): Promise<{ maximized: boolean }>
  windowClose(): Promise<void>
  windowIsMaximized(): Promise<boolean>
  onWindowMaximized(callback: (state: { maximized: boolean }) => void): () => void
  appUpdateStatus(): Promise<AppUpdateStatus>
  checkAppUpdate(): Promise<AppUpdateStatus>
  downloadAppUpdate(): Promise<AppUpdateStatus>
  installAppUpdate(): Promise<void>
  /** Opens only this app's official GitHub Releases page. */
  openAppReleases(): Promise<void>
  onAppUpdate(callback: (status: AppUpdateStatus) => void): () => void
  loadData(): Promise<AppData>
  saveSettings(settings: Settings): Promise<Settings>
  addAccount(username: string): Promise<AppData>
  removeAccount(username: string): Promise<AppData>
  syncGames(username?: string): Promise<AppData>
  /** Full PGN of one game; list rows omit it to keep the payload small. */
  gamePgn(account: string, id: string): Promise<string | null>
  /** Last profile data saved locally (no network); null fields when never fetched. */
  cachedProfile(username: string): Promise<{
    profile: LichessUser | null
    ratingHistory: LichessRatingHistory | null
    profileFetchedAt?: number
  }>
  profile(username: string): Promise<LichessUser>
  ratingHistory(username: string): Promise<LichessRatingHistory>
  /** Sign in through the browser. `username` is the account Lichess authorised. */
  connectLichess(): Promise<{ data: AppData; username: string }>
  engineStatus(): Promise<EngineStatus>
  chooseEngine(): Promise<string | null>
  /**
   * Download and verify the latest official Stockfish build into KChess's own folder.
   * `updated` is false when the installed copy was already the latest and nothing was downloaded.
   */
  installEngine(): Promise<{ path: string; version: string; updated: boolean }>
  /** Delete the downloaded engine; the bundled and chosen ones are never touched. */
  deleteEngine(): Promise<void>
  bestMove(moves: string[], level: EngineLevel, options?: BestMoveOptions): Promise<string>
  startOnline(options: OnlineOptions): Promise<{ id?: string; url?: string; seeking?: boolean }>
  /** Reattach to a game in progress on any connected account. */
  resumeOnline(): Promise<{ id: string; account: string } | null>
  cancelOnline(): Promise<void>
  playOnline(id: string, move: string): Promise<void>
  onlineAction(id: string, action: OnlineAction): Promise<void>
  /** Delete the games and cached profile downloaded for one account, keeping the account. */
  clearAccountData(username: string): Promise<AppData>
  /** Players the connected accounts follow on Lichess (needs the follow permission). */
  following(): Promise<FollowingReport>
  /** Add players as friends without downloading their games. */
  addFriends(usernames: string[]): Promise<AppData>
  /** Data downloaded from Lichess per account, and what is stored locally. */
  usage(): Promise<UsageReport>
  resetUsage(): Promise<void>
  /** Online/playing/signal of the given users, for the game in progress. */
  presence(usernames: string[]): Promise<PresenceReport>
  /** Show a desktop notification if Settings allow it for this kind and the window's state. */
  notify(request: NotificationRequest): Promise<NotificationResult>
  /** Custom themes from the themes folder (created on first use). */
  loadThemes(): Promise<CustomThemeReport>
  openThemesFolder(): Promise<void>
  /** Open the operating system's notification settings (macOS and Windows); false where unsupported. */
  openNotificationSettings(): Promise<boolean>
  /** The microphone permission; with `request`, asks the OS (shows its prompt) when undecided. */
  microphoneAccess(request: boolean): Promise<MicrophoneAccess>
  /** Downloads the verified English model on first use, then reuses the local cache. */
  ensureVoiceModel(): Promise<string>
  onVoiceModelProgress(callback: (progress: VoiceModelProgress) => void): () => void
  /** Open the operating system's microphone privacy settings; false where unsupported. */
  openMicrophoneSettings(): Promise<boolean>
  /** Alerts for the window to show itself, sent instead of a system notification while KChess is in use. */
  onNotification(callback: (alert: { title: string; body: string }) => void): () => void
  onOnlineEvent(callback: (event: OnlineEvent) => void): () => void
  onOnlineError(callback: (message: string) => void): () => void

  /** Lichess puzzle training. Rated results change the account's Lichess puzzle rating. */
  puzzleNext(request: PuzzleRequest): Promise<PuzzleDraw | NeedsReconnect>
  /** Report a puzzle result to Lichess (needs `puzzle:write`). */
  puzzleSolve(request: PuzzleSolveRequest): Promise<PuzzleSolveResult | NeedsReconnect>
  /** The daily puzzle; public, nothing is recorded. */
  puzzleDaily(): Promise<Puzzle>
  /** Read-only: the account's puzzle dashboard. */
  puzzleDashboard(account: string, days: number): Promise<PuzzleDashboard | NeedsReconnect>
  /** Read-only: the account's most recent puzzle attempts. */
  puzzleActivity(account: string, max: number): Promise<PuzzleActivityEntry[] | NeedsReconnect>
  /** Read-only: anyone's public Storm dashboard. */
  stormDashboard(username: string, days: number): Promise<StormDashboard>

  /** Local puzzle database (downloaded from database.lichess.org). */
  puzzleDbStatus(): Promise<PuzzleDbStatus>
  puzzleDbInstall(): Promise<PuzzleDbStatus>
  puzzleDbCancel(): Promise<void>
  puzzleDbDelete(): Promise<PuzzleDbStatus>
  localPuzzles(query: LocalPuzzleQuery): Promise<Puzzle[]>
  /** Puzzles of rising difficulty, for Storm, Streak and Rush. */
  localLadder(query: LocalLadderQuery): Promise<Puzzle[]>
  onPuzzleDbProgress(callback: (progress: PuzzleDbProgress) => void): () => void

  /** Local scores (Storm, Streak, Rush and the practice drills). They never leave this computer. */
  saveRun(run: RunInput): Promise<RunSaved>
  runSummary(kind: RunKind): Promise<RunSummary>
  clearRuns(kind?: RunKind): Promise<void>

  /** Local log of voice input (Settings › Voice). It never leaves this computer unless exported. */
  saveVoiceAttempt(attempt: VoiceAttemptInput): Promise<number>
  updateVoiceAttempt(id: number, update: VoiceAttemptUpdate): Promise<void>
  /** Newest first. */
  voiceHistory(limit: number): Promise<VoiceAttempt[]>
  clearVoiceHistory(): Promise<void>
  /** Save the whole log as JSON where the native dialog says; false when cancelled. */
  exportVoiceHistory(): Promise<boolean>
}
