/**
 * The Lichess service, as the frontends call it. The Rust core owns the operations (`lichess/`
 * in `crates/kchess-core`): the client, accounts and syncs, puzzles, and the online game session.
 * These wrappers keep the names and results the services and tests use.
 */
import type {
  AppData,
  ChallengeInfo,
  ChatLine,
  ChatRoom,
  Crosstable,
  DeclineReason,
  FollowingReport,
  LichessGame,
  LichessRatingHistory,
  LichessUser,
  NeedsReconnect,
  OAuthPageLook,
  OnlineAction,
  OnlineConnection,
  OnlineEvent,
  OnlineOptions,
  OngoingGame,
  PerfStats,
  PerfType,
  PresenceReport,
  Puzzle,
  PuzzleActivityEntry,
  PuzzleDashboard,
  PuzzleDraw,
  PuzzleRequest,
  PuzzleSolveRequest,
  PuzzleSolveResult,
  StoredReview,
  StormDashboard,
} from '../contracts/types'
import { nativeCall, nativeCallSync, onNativeEvent } from './nativeCore'

export interface OnlineHooks {
  /** A game started or finished that is not shown on the board (correspondence, other account). */
  ongoingChanged?: () => void
  /** Health of the idle event stream that waits for challenges and pairings. */
  lobbyState?: (state: {
    account: string
    phase: OnlineConnection['phase']
    message?: string
  }) => void
}

/** Operations on the saved profile cache and the account syncs. */
export function invalidateLogin(accounts: string[]): void {
  nativeCallSync('lichess.invalidateLogin', accounts)
}

export function cancelAccountSyncs(accounts: string[]): void {
  nativeCallSync('lichess.cancelAccountSyncs', accounts)
}

export function forgetProfile(username: string): void {
  nativeCallSync('lichess.forgetProfile', username)
}

export function cachedProfile(username: string): Promise<{
  profile: LichessUser | null
  ratingHistory: LichessRatingHistory | null
  /** When the saved profile was fetched, so callers can skip refreshing a recent one. */
  profileFetchedAt?: number
}> {
  return nativeCall('lichess.cachedProfile', username)
}

export function followedUsers(): Promise<FollowingReport> {
  return nativeCall('lichess.followedUsers')
}

export function primeProfiles(usernames: string[]): Promise<void> {
  return nativeCall('lichess.primeProfiles', usernames)
}

export function profile(username: string): Promise<LichessUser> {
  return nativeCall('lichess.profile', username)
}

export function playerPerf(username: string, perf: PerfType): Promise<PerfStats> {
  return nativeCall('lichess.playerPerf', username, perf)
}

export function exportGame(id: string): Promise<string> {
  return nativeCall('lichess.exportGame', id)
}

export function recentGames(username: string, rated = false): Promise<LichessGame[]> {
  return nativeCall('lichess.recentGames', username, rated)
}

export function sendMessage(
  account: string,
  username: string,
  text: string,
): Promise<{ sent: true } | NeedsReconnect> {
  return nativeCall('lichess.sendMessage', account, username, text)
}

export function crosstable(a: string, b: string): Promise<Crosstable> {
  return nativeCall('lichess.crosstable', a, b)
}

export function ratingHistory(username: string): Promise<LichessRatingHistory> {
  return nativeCall('lichess.ratingHistory', username)
}

/** Lichess's analysis of games to review, stored as reviews. */
export function fetchLichessReviews(
  account: string,
  ids: readonly string[],
): Promise<StoredReview[]> {
  if (!ids.length) return Promise.resolve([])
  return nativeCall('lichess.fetchLichessReviews', account, [...ids])
}

export function syncGames(username?: string): Promise<AppData> {
  return nativeCall('lichess.syncGames', username ?? null)
}

/** Signs an account in through the browser; the window is brought forward when it returns. */
export function connectLichess(look?: OAuthPageLook): Promise<{ data: AppData; username: string }> {
  return nativeCall('lichess.connectLichess', look ?? null)
}

export function puzzleNext(request: PuzzleRequest): Promise<PuzzleDraw | NeedsReconnect> {
  return nativeCall('lichess.puzzleNext', request)
}

export function puzzleSolve(
  request: PuzzleSolveRequest,
): Promise<PuzzleSolveResult | NeedsReconnect> {
  return nativeCall('lichess.puzzleSolve', request)
}

export function puzzleDaily(): Promise<Puzzle> {
  return nativeCall('lichess.puzzleDaily')
}

export function puzzleDashboard(
  account: string,
  days: number,
): Promise<PuzzleDashboard | NeedsReconnect> {
  return nativeCall('lichess.puzzleDashboard', { account, days })
}

export function puzzleActivity(
  account: string,
  max: number,
): Promise<PuzzleActivityEntry[] | NeedsReconnect> {
  return nativeCall('lichess.puzzleActivity', account, max)
}

/** Anyone's public Storm results; needs no login. */
export function stormDashboard(username: string, days: number): Promise<StormDashboard> {
  return nativeCall('lichess.stormDashboard', username, days)
}

/**
 * The online game session of the signed-in accounts. The Rust session owns the state; its events
 * (`online:state`, `online:event`, `online:error`, `online:lobby`, `online:ongoing-changed`,
 * `challenge:received`) reach the callbacks given here.
 */
export class OnlineSession {
  private readonly unsubscribe: Array<() => void> = []

  constructor(
    emit: (event: OnlineEvent) => void,
    error: (error: string) => void,
    state: (state: OnlineConnection) => void = () => {},
    hooks: OnlineHooks = {},
  ) {
    this.unsubscribe.push(
      onNativeEvent<OnlineEvent>('online:event', emit),
      onNativeEvent<string>('online:error', error),
      onNativeEvent<OnlineConnection>('online:state', state),
      onNativeEvent<{ account: string; phase: OnlineConnection['phase']; message?: string }>(
        'online:lobby',
        (lobby) => hooks.lobbyState?.(lobby),
      ),
      onNativeEvent<null>('online:ongoing-changed', () => hooks.ongoingChanged?.()),
    )
  }

  /** Whether a live human game is being played. */
  get playing(): boolean {
    return nativeCallSync<boolean>('online.playing')
  }

  /** Whether engine, review and explorer assistance is blocked by a live game. */
  get assistanceBlocked(): boolean {
    return nativeCallSync<boolean>('online.assistanceBlocked')
  }

  stayConnected(account: string): Promise<void> {
    return nativeCall('online.stayConnected', account)
  }

  attach(account: string, id: string): Promise<void> {
    return nativeCall('online.attach', account, id)
  }

  resume(): Promise<{ id: string; account: string } | null> {
    return nativeCall('online.resume')
  }

  ongoing(): Promise<OngoingGame[]> {
    return nativeCall('online.ongoing')
  }

  open(account: string, id: string): Promise<void> {
    return nativeCall('online.open', account, id)
  }

  start(
    options: OnlineOptions,
  ): Promise<{ id?: string; url?: string; seeking?: boolean; correspondence?: boolean }> {
    return nativeCall('online.start', options)
  }

  acceptChallenge(challenge: ChallengeInfo): Promise<void> {
    return nativeCall('online.acceptChallenge', challenge)
  }

  declineChallenge(challenge: ChallengeInfo, reason: DeclineReason): Promise<void> {
    return nativeCall('online.declineChallenge', challenge, reason)
  }

  withdrawChallenge(challenge: ChallengeInfo): Promise<void> {
    return nativeCall('online.withdrawChallenge', challenge)
  }

  move(id: string, uci: string): Promise<void> {
    return nativeCall('online.move', id, uci)
  }

  chat(id: string): Promise<ChatLine[]> {
    return nativeCall('online.chat', id)
  }

  sendChat(id: string, room: ChatRoom, text: string): Promise<void> {
    return nativeCall('online.sendChat', id, room, text)
  }

  presence(usernames: string[]): Promise<PresenceReport> {
    return nativeCall('online.presence', usernames)
  }

  action(id: string, action: OnlineAction): Promise<void> {
    return nativeCall('online.action', id, action)
  }

  /** Ends the attachments, pending challenge and streams of the session. */
  cancel(): Promise<void> {
    return nativeCall('online.cancel')
  }

  logout(accounts?: string[]): Promise<void> {
    return nativeCall('online.logout', accounts ?? null)
  }

  /** Stops the session for good: its streams and lobby. */
  close(): Promise<void> {
    for (const stop of this.unsubscribe.splice(0)) stop()
    return nativeCall('online.close')
  }
}

/** Forgets every cached profile, sync and login epoch of the Lichess service. */
export function resetLichess(): Promise<void> {
  return nativeCall('lichess.reset')
}
