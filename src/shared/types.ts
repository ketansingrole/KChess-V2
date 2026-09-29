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
export const ONLINE_ACTIONS = ['resign', 'abort', 'takeback', 'declineTakeback'] as const
export type OnlineAction = (typeof ONLINE_ACTIONS)[number]
export const CHALLENGE_COLORS = ['random', 'white', 'black'] as const
export type ChallengeColor = (typeof CHALLENGE_COLORS)[number]

export interface Settings {
  appearance: Appearance
  boardTheme: string
  coordinates: CoordinateMode
  soundEnabled: boolean
  soundVolume: number
  enginePath: string
  /** Let the player queue a move while it is the opponent's turn. */
  premove: boolean
  promotion: PromotionMode
  /** Dot the squares a selected piece can move to. */
  showLegalMoves: boolean
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
export type UsageKind = 'games' | 'profile' | 'play' | 'presence' | 'other'

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

export interface DesktopApi {
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
  bestMove(moves: string[], level: EngineLevel): Promise<string>
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
  onOnlineEvent(callback: (event: OnlineEvent) => void): () => void
  onOnlineError(callback: (message: string) => void): () => void
}
