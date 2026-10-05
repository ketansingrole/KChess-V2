import type { components, paths } from '@lichess-org/types'
import type { Variant } from './variant'

export type { Variant }

export const APPEARANCES = ['system', 'light', 'dark'] as const
export type Appearance = (typeof APPEARANCES)[number]
export const COORDINATE_MODES = ['none', 'inside', 'outside'] as const
export type CoordinateMode = (typeof COORDINATE_MODES)[number]
export const ENGINE_LEVELS = [
  'beginner',
  'novice',
  'casual',
  'club',
  'strong-club',
  'expert',
  'cm',
  'fm',
  'im',
  'gm',
  'super-gm',
  'max',
] as const
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
  'challenge',
  'test',
] as const
/** What a desktop notification is about; each kind (except `test`) has its own switch in Settings. */
export type NotificationKind = (typeof NOTIFICATION_KINDS)[number]
export const ONLINE_ACTIONS = [
  'resign',
  'abort',
  'takeback',
  'declineTakeback',
  'offerDraw',
  'acceptDraw',
  'declineDraw',
  'claimVictory',
  'claimDraw',
  'berserk',
] as const
export type OnlineAction = (typeof ONLINE_ACTIONS)[number]
/** Days per move Lichess allows for correspondence games. */
export const CORRESPONDENCE_DAYS = [1, 2, 3, 5, 7, 10, 14] as const
export type CorrespondenceDays = (typeof CORRESPONDENCE_DAYS)[number]
/** Reasons Lichess accepts when declining a challenge (it translates them for the challenger). */
export const DECLINE_REASONS = [
  'generic',
  'later',
  'tooFast',
  'tooSlow',
  'timeControl',
  'rated',
  'casual',
  'standard',
  'variant',
] as const
export type DeclineReason = (typeof DECLINE_REASONS)[number]
export const CHAT_ROOMS = ['player', 'spectator'] as const
export type ChatRoom = (typeof CHAT_ROOMS)[number]
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
export const VOICE_SOURCES = ['computer', 'coordinates', 'analysis', 'editor', 'local'] as const
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

/** Resolved CSS colors the browser's Lichess sign-in page uses to match the app. */
export interface OAuthPageColors {
  bg: string
  elevated: string
  text: string
  textMuted: string
  primary: string
  border: string
}

export interface OAuthPageLook {
  appearance: Appearance
  light: OAuthPageColors
  dark: OAuthPageColors
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
  /** Computer levels offered in Play with Computer, in ladder order; never empty. */
  engineLevels: EngineLevel[]
  /** Review your games in the background: none, the last 30 days', or all of them. */
  reviewAuto: ReviewAuto
  /** Keep reviewing in the background on battery power. */
  reviewOnBattery: boolean
  /** Keep the Lichess event stream open while idle, so challenges (and tournament pairings) arrive. */
  receiveChallenges: boolean
  /** Alert when someone challenges you. */
  notifyChallenges: boolean
  /** Show the player chat in online games. */
  onlineChat: boolean
  /** Minutes between checks of correspondence games for your turn; 0 turns checking off. */
  correspondencePoll: number
  /** Hide everything but the board, clocks and essential controls while playing. */
  zenMode: boolean
  /** Hide the pieces in games you play (the move list and clocks stay). */
  blindfold: boolean
  /** Ask Lichess's cloud for a cached evaluation of analysis positions (sends the position). */
  cloudEval: boolean
  /** Name the opening of the position in games and analysis. */
  showOpeningName: boolean
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
  gameCount: number
}

export interface GamePageQuery {
  account?: string
  result?: 'win' | 'loss' | 'draw'
  rated?: boolean
  offset: number
  limit: number
}

export interface GamePage {
  games: LichessGame[]
  total: number
}

export interface GameRecord {
  total: number
  win: number
  loss: number
  draw: number
}

export interface InsightsQuery {
  account: string
  speed?: string
  rated?: boolean
  /** Only games from the last this-many days. */
  days?: number
}

/** Patterns in one account's synced games (worked out locally). */
export interface InsightsReport {
  account: string
  total: number
  record: GameRecord
  byColor: { white: GameRecord; black: GameRecord }
  bySpeed: { speed: string; record: GameRecord }[]
  /** Opening families, most played first. */
  byOpening: { name: string; record: GameRecord; asWhite: number }[]
  /** Sunday first, in this computer's time zone. */
  byWeekday: GameRecord[]
  byHour: GameRecord[]
  /** By the opponent's rating minus yours. */
  byOpponent: { label: string; record: GameRecord }[]
  byLength: { label: string; record: GameRecord }[]
  /** How games ended (`mate`, `resign`, `outoftime`, `draw`…). */
  endings: { status: string; record: GameRecord }[]
  streaks: { longestWin: number; longestLoss: number; current: number }
  /** From reviewed games only. */
  accuracy?: {
    games: number
    average?: number
    acpl?: number
    perGame: { inaccuracy: number; mistake: number; blunder: number }
  }
}

export interface GameLibraryOverview {
  byAccount: Record<string, GameRecord>
  versus: Record<string, GameRecord>
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

export interface OnlineConnection {
  session: number
  account: string
  gameId: string
  lane: 'events' | 'game' | 'seek'
  phase:
    | 'checking'
    | 'connecting'
    | 'connected'
    | 'reconnecting'
    | 'disconnected'
    | 'auth-required'
    | 'idle'
  message?: string
}

/** A player as a challenge or ongoing game names them. */
export interface PlayerRef {
  name: string
  rating?: number
  title?: string
  provisional?: boolean
  online?: boolean
}

export type ChallengeTimeControl =
  | { type: 'clock'; limit: number; increment: number }
  | { type: 'correspondence'; days: number }
  | { type: 'unlimited' }

/** A pending challenge to or from one of the connected accounts. */
export interface ChallengeInfo {
  id: string
  /** The connected account it was sent to (incoming) or from (outgoing). */
  account: string
  direction: 'in' | 'out'
  /** The other player. */
  opponent: PlayerRef
  /** Lichess variant key (`standard`, `chess960`, `crazyhouse`…). */
  variant: string
  variantName: string
  rated: boolean
  speed: string
  timeControl: ChallengeTimeControl
  /** The colour the challenger asked for. */
  color: ChallengeColor
  /** A rematch offer for this game. */
  rematchOf?: string
  initialFen?: string
  /** Lichess says KChess (a Board API client) can play this game, and KChess knows its variant. */
  playable: boolean
  /** Why it cannot be accepted here, when it cannot. */
  problem?: string
  receivedAt: number
}

/** Health of the idle event stream that waits for challenges and pairings. */
export interface LobbyState {
  account: string
  phase: OnlineConnection['phase']
  message?: string
}

export interface ChatLine {
  user: string
  text: string
  room: ChatRoom
}

/** A game a connected account is playing, real-time or correspondence. */
export interface OngoingGame {
  gameId: string
  account: string
  opponent: PlayerRef
  color: 'white' | 'black'
  fen: string
  lastMove?: string
  isMyTurn: boolean
  /** Seconds left on your clock (or for your correspondence move), when Lichess says. */
  secondsLeft?: number
  variant: string
  speed: string
  rated: boolean
  tournamentId?: string
}

export type TournamentSystem = 'arena' | 'swiss'

/** An arena or Swiss tournament in a list. */
export interface TournamentSummary {
  id: string
  system: TournamentSystem
  name: string
  status: 'created' | 'started' | 'finished'
  variant: string
  variantName: string
  rated: boolean
  /** Seconds and seconds per move. */
  clock: { limit: number; increment: number }
  /** Length of an arena in minutes; Swiss events have rounds instead. */
  minutes?: number
  round?: number
  nbRounds?: number
  nbPlayers: number
  startsAt: number
  finishesAt?: number
  /** The team a Swiss event belongs to. */
  team?: { id: string; name: string }
  /** The Board API can play its games and KChess knows its variant. */
  playable: boolean
  problem?: string
}

export interface TournamentStanding {
  rank: number
  name: string
  title?: string
  rating?: number
  score?: number
  /** Arena score sheet: one digit per game, newest last. */
  sheet?: string
  fire?: boolean
}

export interface TournamentDetail extends TournamentSummary {
  description?: string
  secondsToStart?: number
  secondsToFinish?: number
  berserkable?: boolean
  standing: TournamentStanding[]
  /** You, when the account you asked as has joined. */
  me?: { rank?: number; withdraw?: boolean; gameId?: string }
  /** Entry conditions and whether the account meets them. */
  verdicts?: { accepted: boolean; list: { condition: string; verdict: string }[] }
  /** Swiss: when the next round starts. */
  nextRoundIn?: number
}

export interface TournamentList {
  arenas: TournamentSummary[]
  swiss: TournamentSummary[]
  /** Teams of the account whose Swiss events could not be read, and other notes. */
  problems: string[]
}

/* ── Players ──────────────────────────────────────────────────────────── */

export const PERF_TYPES = [
  'ultraBullet',
  'bullet',
  'blitz',
  'rapid',
  'classical',
  'correspondence',
  'chess960',
  'kingOfTheHill',
  'threeCheck',
  'antichess',
  'atomic',
  'horde',
  'racingKings',
  'crazyhouse',
] as const
export type PerfType = (typeof PERF_TYPES)[number]

export interface PerfResult {
  opponent: string
  opponentRating: number
  at: string
  gameId: string
}

/** One player's record in one rating category, as Lichess's perf page shows it. */
export interface PerfStats {
  perf: PerfType
  rating?: number
  deviation?: number
  provisional?: boolean
  /** Position on the leaderboard, when ranked. */
  rank?: number
  /** Better than this share of players. */
  percentile?: number
  progress?: number
  count: {
    all: number
    rated: number
    win: number
    loss: number
    draw: number
    tour: number
    berserk: number
    opAvg: number
    seconds: number
    disconnects: number
  }
  highest?: { rating: number; at: string; gameId: string }
  lowest?: { rating: number; at: string; gameId: string }
  bestWins: PerfResult[]
  worstLosses: PerfResult[]
  winStreak: { current: number; best: number }
  lossStreak: { current: number; best: number }
}

/** Two players' lifetime score against each other (and in their current match, if any). */
export interface Crosstable {
  /** Lower-cased username → points (wins plus half-points for draws). */
  users: Record<string, number>
  nbGames: number
  matchup?: { users: Record<string, number>; nbGames: number }
}

/* ── Watching games ─────────────────────────────────────────────────── */

export interface WatchPlayer {
  name: string
  title?: string
  rating?: number
}

/** Connection health of the TV/game feed, scoped to its watch session. */
export interface WatchState {
  session: number
  phase: 'connecting' | 'connected' | 'ended' | 'error'
  message?: string
}

/** One position of a game being watched (TV or a game by id). */
export interface WatchFrame {
  /** Which `watch` call it belongs to; frames of an earlier one are stale. */
  session: number
  source: 'tv' | 'game'
  channel?: string
  gameId: string
  white: WatchPlayer
  black: WatchPlayer
  orientation: 'white' | 'black'
  /** Board placement plus side to move (pockets of Crazyhouse are dropped). */
  fen: string
  lastMove?: string
  /** Clocks in seconds when the frame was sent. */
  whiteClock?: number
  blackClock?: number
  variant: string
  speed?: string
  status?: string
  winner?: 'white' | 'black'
  finished: boolean
}

export interface TvChannel {
  key: string
  label: string
  gameId?: string
  player?: WatchPlayer
}

export interface BroadcastSummary {
  tourId: string
  tourName: string
  description?: string
  image?: string
  roundId?: string
  roundName?: string
  ongoing: boolean
  startsAt?: number
  section: 'active' | 'upcoming' | 'past'
}

export interface BroadcastRoundRef {
  id: string
  name: string
  ongoing: boolean
  finished: boolean
  startsAt?: number
}

export interface BroadcastTourDetail {
  id: string
  name: string
  description?: string
  rounds: BroadcastRoundRef[]
  defaultRoundId?: string
}

/** One board of a broadcast round, as the live PGN feed last described it. */
export interface BroadcastGame {
  /** The study chapter id. */
  id: string
  name: string
  white: WatchPlayer
  black: WatchPlayer
  result: string
  startFen: string
  /** UCI moves of the main line. */
  moves: string[]
  fen: string
  lastMove?: string
  /** Clocks in seconds, from the last comments of each side. */
  whiteClock?: number
  blackClock?: number
  ongoing: boolean
  /** The game as PGN, for the analysis board. */
  pgn: string
}

export interface BroadcastUpdate {
  session: number
  roundId: string
  /** Games that changed (all of them at first). */
  games: BroadcastGame[]
  /** The feed ended (the round is over or the connection dropped). */
  ended?: boolean
  error?: string
}

export interface EngineStatus {
  /** Identity of the executable currently selected, including native file changes. */
  identity?: string
  ready: boolean
  /** The executable in use (a downloaded or chosen one), or empty when the bundled engine is. */
  path: string
  bundled: boolean
  /** The Stockfish KChess downloaded, whether or not it is the one in use. */
  managed: { installed: boolean; path: string; version?: string }
  /** An official native download exists for this platform and architecture. */
  canDownload: boolean
}

export interface OnlineOptions {
  /** Ignored for correspondence games (`days`). */
  minutes: number
  increment: number
  /** Correspondence: days per move instead of a clock. */
  days?: CorrespondenceDays
  /** Lichess variant; standard when omitted. */
  variant?: Variant
  /** Start a direct challenge from this position (standard or Chess960 rules). */
  fen?: string
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
export type UsageKind =
  | 'games'
  | 'profile'
  | 'play'
  | 'presence'
  | 'puzzles'
  | 'database'
  | 'watch'
  | 'tournament'
  | 'analysis'
  | 'study'
  | 'other'

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
  /** The game they are playing, when Lichess says (watchable from the Watch page). */
  playingId?: string
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

export interface VoiceModelStatus {
  installed: boolean
  bytes: number
  busy: boolean
  progress?: VoiceModelProgress
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
  /** Chess960 rules: castling is king-takes-rook in UCI. */
  chess960?: boolean
}

/** Ask the analysis engine to study one position until stopped or a limit is reached. */
export interface AnalysisRequest {
  fen: string
  /** Repetition context: root plus moves must produce fen. */
  rootFen?: string
  moves?: string[]
  clientId?: number
  /** How many of the best lines to report (MultiPV). */
  lines: number
  /** Keep searching until stopped instead of stopping at a sensible depth. */
  infinite?: boolean
}

/** One engine line. Scores are from White's point of view, like Lichess shows them. */
export interface EngineLine {
  /** 1 for the best line, 2 for the next best… */
  rank: number
  depth: number
  /** Centipawns, when there is no forced mate. */
  cp?: number
  /** Moves to mate: positive when White mates, negative when Black does. */
  mate?: number
  /** Principal variation as UCI moves. */
  pv: string[]
}

export interface AnalysisUpdate {
  /** The request this belongs to; the renderer drops updates of superseded ones. */
  id: number
  fen: string
  depth: number
  /** Nodes per second, for the engine's status line. */
  nps?: number
  lines: EngineLine[]
  /** The search finished (limit reached, or stopped) and no more updates follow. */
  done: boolean
  context?: string
  clientId?: number
  reason?: 'completed' | 'interrupted' | 'failed'
  engine?: string
  error?: string
}

/* ── Game review ───────────────────────────────────────────────────── */

export const JUDGMENTS = ['inaccuracy', 'mistake', 'blunder'] as const
export type Judgment = (typeof JUDGMENTS)[number]

/**
 * One position of a reviewed game: its score from White's side and the engine's choice there.
 * A checkmated position is `mate: 0` (the side to move has lost).
 */
export interface ReviewEval {
  cp?: number
  mate?: number
  /** The engine's move in this position (UCI), and the line it expects after it. */
  best?: string
  pv?: string[]
  depth?: number
}

export type ReviewSource = 'lichess' | 'local'

/** A game's review as stored: the raw scores; labels and accuracy are worked out from them. */
export interface StoredReview {
  /** `reviewKey(fen, moves)`: the same moves from the same start share one review. */
  key: string
  fen: string
  /** UCI, from `fen`. */
  moves: string[]
  source: ReviewSource
  engine?: string
  /** Index 0 is the starting position, index i the position after move i; null until analysed. */
  evals: (ReviewEval | null)[]
  /** Lichess's own label for each move (index i is move i+1), when Lichess did the analysis. */
  judgments?: (Judgment | null)[]
  /** Lichess's own accuracy figures, when Lichess did the analysis. */
  accuracy?: { white?: number; black?: number }
  /** Depth the local analysis aims for; 0 for Lichess's analysis. */
  depth: number
  complete: boolean
  updatedAt: number
  /** The Lichess game it belongs to, if any. */
  gameId?: string
}

/** One player's side of a review. */
export interface ReviewSide {
  accuracy?: number
  /** Average centipawn loss. */
  acpl?: number
  inaccuracy: number
  mistake: number
  blunder: number
}

/** What the game list shows for a reviewed game. */
export interface ReviewSummary {
  key: string
  source: ReviewSource
  engine?: string
  complete: boolean
  white: ReviewSide
  black: ReviewSide
}

export const REVIEW_AUTO = ['off', 'recent', 'all'] as const
export type ReviewAuto = (typeof REVIEW_AUTO)[number]

export interface ReviewRequest {
  fen: string
  moves: string[]
  /** A Lichess game: its own analysis is fetched first when it has one. */
  gameId?: string
  account?: string
}

/** Progress of the review queue, sent whenever it changes. */
export interface ReviewStatus {
  /** The review being worked on, with how many positions are done. */
  current?: { key: string; gameId?: string; done: number; total: number; background: boolean }
  /** Reviews waiting, asked for and automatic. */
  waiting: number
  /** Why automatic reviews are on hold, when they are. */
  paused?: 'battery' | 'engine' | 'online' | 'off'
  /** The last review asked for that could not be done, and why. */
  failed?: { key: string; message: string }
}

export interface ReviewUpdate {
  review: StoredReview
  summary: ReviewSummary
}

export const POSITION_LOOKUP_KINDS = ['opening', 'masters', 'player', 'tablebase'] as const
/** `opening` is the Lichess games database, `masters` over-the-board master games, `player` one player's games. */
export type PositionLookupKind = (typeof POSITION_LOOKUP_KINDS)[number]
export const EXPLORER_SPEEDS = [
  'ultraBullet',
  'bullet',
  'blitz',
  'rapid',
  'classical',
  'correspondence',
] as const
export type ExplorerSpeed = (typeof EXPLORER_SPEEDS)[number]
export const EXPLORER_RATINGS = [400, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2500] as const
/** Filters of the opening explorer; each database uses the ones it understands. */
export interface LookupOptions {
  /** Lichess and player databases. */
  speeds?: ExplorerSpeed[]
  /** Lichess database: rating groups (each is the lower bound of its band). */
  ratings?: number[]
  /** Player database: whose games, and with which colour. */
  player?: string
  color?: 'white' | 'black'
  /** Player database: rated, casual or both. */
  modes?: ('rated' | 'casual')[]
  /** Games since this month (`YYYY-MM`; masters use a year). */
  since?: string
}
/** A game the explorer lists as notable for the position. */
export interface ExplorerGame {
  id: string
  white: string
  black: string
  whiteRating?: number
  blackRating?: number
  winner?: 'white' | 'black'
  year?: number
  month?: string
  /** The move played from this position, as SAN. */
  san?: string
}
export interface PositionLookup {
  kind: PositionLookupKind
  fen: string
  /** Games in the database reaching this position (opening databases). */
  total?: number
  games?: ExplorerGame[]
  fetchedAt: number
  cached: boolean
  stale: boolean
  message?: string
  opening?: string
  category?: string
  dtz?: number | null
  moves: {
    uci: string
    san: string
    white?: number
    draws?: number
    black?: number
    category?: string
    dtz?: number | null
  }[]
}

export interface LichessStudy {
  id: string
  name: string
  updatedAt: number
}
export interface LichessStudyChapter {
  name: string
  pgn: string
}

export interface ExportRequest {
  /** File name without extension. */
  name: string
  kind: 'gif' | 'png' | 'pgn'
  data: Uint8Array | string
}

/** A cloud evaluation: lines from White's point of view, like the local engine's. */
export interface CloudEval {
  fen: string
  depth: number
  knodes: number
  lines: EngineLine[]
}

export interface DesktopApi {
  recordPerformance(
    name: 'app.ready' | 'board.frame' | 'voice.activation',
    milliseconds: number,
  ): Promise<void>
  positionLookup(
    kind: PositionLookupKind,
    fen: string,
    options?: LookupOptions,
  ): Promise<PositionLookup>
  /**
   * Save an exported game (an animated GIF, a PNG of a position, or PGN text) where the native
   * dialog says; false when cancelled.
   */
  saveExport(request: ExportRequest): Promise<boolean>
  /** Studies an account owns or belongs to on Lichess. */
  lichessStudies(account: string): Promise<LichessStudy[] | NeedsReconnect>
  lichessStudyChapters(account: string, id: string): Promise<LichessStudyChapter[] | NeedsReconnect>
  /** Add a PGN as a chapter of a Lichess study (a new private one when `studyId` is empty). */
  exportToLichessStudy(
    account: string,
    studyId: string,
    name: string,
    pgn: string,
  ): Promise<{ id: string } | NeedsReconnect>
  /** A finished Lichess game's PGN, for the analysis board. */
  exportGame(id: string): Promise<string>
  /** A master game's PGN from the masters database, for the analysis board. */
  mastersGame(id: string): Promise<string>
  /** Lichess's cached cloud evaluation of a position, or null when it has none. */
  cloudEval(fen: string, lines: number): Promise<CloudEval | null>
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
  logout(username: string): Promise<AppData>
  logoutAll(): Promise<AppData>
  removeAccount(username: string): Promise<AppData>
  syncGames(username?: string): Promise<AppData>
  /** Filtered, bounded library rows; full PGN is fetched separately. */
  gamePage(query: GamePageQuery): Promise<GamePage>
  gameLibraryOverview(): Promise<GameLibraryOverview>
  /** Patterns in an account's synced games, worked out on this computer. */
  insights(query: InsightsQuery): Promise<InsightsReport>
  gameRatingHistory(account: string): Promise<LichessRatingHistory>
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
  connectLichess(look?: OAuthPageLook): Promise<{ data: AppData; username: string }>
  engineStatus(): Promise<EngineStatus>
  chooseEngine(): Promise<string | null>
  /**
   * Download and verify the latest official Stockfish build into KChess's own folder.
   * `updated` is false when the installed copy was already the latest and nothing was downloaded.
   */
  installEngine(): Promise<{ path: string; version: string; updated: boolean }>
  /** Delete the downloaded engine; the bundled and chosen ones are never touched. */
  deleteEngine(): Promise<void>
  stopEngine(): Promise<void>
  bestMove(moves: string[], level: EngineLevel, options?: BestMoveOptions): Promise<string>
  /** Start analysing a position (replacing any running analysis); updates arrive on `onAnalysis`. */
  startAnalysis(request: AnalysisRequest): Promise<number>
  stopAnalysis(): Promise<void>
  onAnalysis(callback: (update: AnalysisUpdate) => void): () => void
  /** The stored review of these moves from this position, if any. */
  reviewGet(fen: string, moves: string[]): Promise<StoredReview | null>
  /**
   * Review a game now, ahead of automatic reviews (a Lichess game is looked up on Lichess first).
   * Returns what is stored so far; the rest arrives on `onReviewUpdate`.
   */
  reviewRequest(request: ReviewRequest): Promise<StoredReview | null>
  reviewCancel(key: string): Promise<void>
  reviewStatus(): Promise<ReviewStatus>
  /** Review summaries of up to one history page of games, by game id. */
  reviewSummaries(ids: string[]): Promise<Record<string, ReviewSummary>>
  onReviewUpdate(callback: (update: ReviewUpdate) => void): () => void
  onReviewStatus(callback: (status: ReviewStatus) => void): () => void
  startOnline(options: OnlineOptions): Promise<{ id?: string; url?: string; seeking?: boolean }>
  /** Reattach to a game in progress on any connected account. */
  resumeOnline(): Promise<{ id: string; account: string } | null>
  cancelOnline(): Promise<void>
  playOnline(id: string, move: string): Promise<void>
  onlineAction(id: string, action: OnlineAction): Promise<void>
  /** The private player chat of the game being played (needs the setting on). */
  onlineChat(id: string): Promise<ChatLine[]>
  sendChat(id: string, room: ChatRoom, text: string): Promise<void>
  /**
   * Keep this connected account's Lichess event stream open while no game is being played, so
   * challenges and tournament pairings arrive; empty closes it.
   */
  stayConnected(account: string): Promise<void>
  /** Pending challenges to and from the connected accounts. */
  challenges(): Promise<ChallengeInfo[]>
  onChallenges(callback: (challenges: ChallengeInfo[]) => void): () => void
  acceptChallenge(id: string): Promise<void>
  declineChallenge(id: string, reason: DeclineReason): Promise<void>
  /** Withdraw a challenge you sent. */
  cancelChallenge(id: string): Promise<void>
  /** Games the connected accounts are playing, most urgent first. */
  ongoingGames(): Promise<OngoingGame[]>
  /** Open one of those games on the online board (a correspondence game, or a live one). */
  openGame(account: string, id: string): Promise<void>
  onLobbyState(callback: (state: LobbyState) => void): () => void
  playerPerf(username: string, perf: PerfType): Promise<PerfStats>
  crosstable(a: string, b: string): Promise<Crosstable>
  /** Lichess TV channels and who is on each. */
  tvChannels(): Promise<TvChannel[]>
  /** Watch a TV channel or any game by id (replacing what was watched); frames on `onWatch`. */
  watch(target: { channel: string } | { gameId: string }): Promise<number>
  /** Follow a broadcast round's live PGN; updates on `onBroadcast`. */
  watchBroadcast(roundId: string): Promise<number>
  stopWatching(): Promise<void>
  onWatchState(callback: (state: WatchState) => void): () => void
  onWatch(callback: (frame: WatchFrame) => void): () => void
  onBroadcast(callback: (update: BroadcastUpdate) => void): () => void
  broadcasts(): Promise<BroadcastSummary[]>
  broadcastTour(id: string): Promise<BroadcastTourDetail>
  /** Current arenas, and the Swiss events of the account's teams. */
  tournaments(account: string): Promise<TournamentList>
  tournament(system: TournamentSystem, id: string, account: string): Promise<TournamentDetail>
  joinTournament(
    system: TournamentSystem,
    id: string,
    account: string,
    password?: string,
  ): Promise<true | NeedsReconnect>
  leaveTournament(
    system: TournamentSystem,
    id: string,
    account: string,
  ): Promise<true | NeedsReconnect>
  /** A game started or ended that the board does not show (refresh `ongoingGames`). */
  onOngoingChanged(callback: () => void): () => void
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
  /** Checks the verified local voice model without downloading or requesting microphone access. */
  voiceModelStatus(): Promise<VoiceModelStatus>
  onVoiceModelProgress(callback: (progress: VoiceModelProgress) => void): () => void
  /** Open the operating system's microphone privacy settings; false where unsupported. */
  openMicrophoneSettings(): Promise<boolean>
  /** Alerts for the window to show itself, sent instead of a system notification while KChess is in use. */
  onNotification(callback: (alert: { title: string; body: string }) => void): () => void
  onOnlineState(callback: (state: OnlineConnection) => void): () => void
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
