/**
 * Input validation for values crossing the renderer → main IPC boundary. The checks are the Rust
 * validators in `crates/kchess-domain/src/misc`; each wrapper returns the normalized value or
 * throws the message the check rejected with.
 */
import {
  ENGINE_LEVELS,
  ONLINE_ACTIONS,
  type LookupOptions,
  type NewArena,
  type PerfType,
  type GamePageQuery,
  type AnalysisRequest,
  type ReviewRequest,
  type BestMoveOptions,
  type VoiceAttemptInput,
  type VoiceAttemptUpdate,
  type EngineLevel,
  type LocalLadderQuery,
  type LocalPuzzleQuery,
  type PuzzleRequest,
  type PuzzleSolveRequest,
  type RunInput,
  type RunKind,
  type AppTheme,
  type NotificationRequest,
  type OnlineAction,
  type OnlineOptions,
  type Settings,
  type CoreSettings,
  type ChatRoom,
  type ExportRequest,
  type InsightsQuery,
  type DeclineReason,
} from '@kchess/contracts/types'
import { rulesLossless } from './engine.ts'
import { FEN, GAME_ID, PUZZLE_ANGLE, PUZZLE_ID, UCI_MOVE, USERNAME } from './patterns.ts'

export { FEN, GAME_ID, PUZZLE_ANGLE, PUZZLE_ID, UCI_MOVE, USERNAME }
export { ENGINE_LEVELS, ONLINE_ACTIONS }
export type { EngineLevel, OnlineAction }

export const assertUsername = (value: unknown): string =>
  rulesLossless<string>('assertUsername', value)
export const assertGameId = (value: unknown): string => rulesLossless<string>('assertGameId', value)
export const assertGameIds = (value: unknown): string[] =>
  rulesLossless<string[]>('assertGameIds', value)
export const assertUci = (value: unknown): string => rulesLossless<string>('assertUci', value)
export const assertMoves = (value: unknown): string[] =>
  rulesLossless<string[]>('assertMoves', value)
export const assertLevel = (value: unknown): EngineLevel =>
  rulesLossless<EngineLevel>('assertLevel', value)
export const assertUsernames = (value: unknown): string[] =>
  rulesLossless<string[]>('assertUsernames', value)
export const assertFriendList = (value: unknown): string[] =>
  rulesLossless<string[]>('assertFriendList', value)
export const assertAction = (value: unknown): OnlineAction =>
  rulesLossless<OnlineAction>('assertAction', value)
export const assertChatRoom = (value: unknown): ChatRoom =>
  rulesLossless<ChatRoom>('assertChatRoom', value)
export const assertChatText = (value: unknown): string =>
  rulesLossless<string>('assertChatText', value)
/** A new arena, limited to the values Lichess accepts. */
export const assertNewArena = (value: unknown): NewArena =>
  rulesLossless<NewArena>('assertNewArena', value)
/** A Lichess private message, which Lichess caps at 8,000 characters. */
export const assertMessageText = (value: unknown): string =>
  rulesLossless<string>('assertMessageText', value)
export const assertDeclineReason = (value: unknown): DeclineReason =>
  rulesLossless<DeclineReason>('assertDeclineReason', value)
export const assertTournamentSystem = (value: unknown): 'arena' | 'swiss' =>
  rulesLossless<'arena' | 'swiss'>('assertTournamentSystem', value)
export const assertTournamentId = (value: unknown): string =>
  rulesLossless<string>('assertTournamentId', value)
/** An empty or missing password is no password. */
export const assertTournamentPassword = (value: unknown): string | undefined =>
  rulesLossless<string | null>('assertTournamentPassword', value) ?? undefined
/** An export to save: the bytes must really be the kind of file they claim to be. */
export function assertExport(value: unknown): ExportRequest {
  const request = rulesLossless<ExportRequest>('assertExport', value)
  // The core checks a binary export's size and magic bytes; the bytes themselves stay the caller's.
  return { ...request, data: (value as ExportRequest).data }
}
export const assertInsightsQuery = (value: unknown): InsightsQuery =>
  rulesLossless<InsightsQuery>('assertInsightsQuery', value)
export const assertPerfType = (value: unknown): PerfType =>
  rulesLossless<PerfType>('assertPerfType', value)
export const assertLichessId = (value: unknown): string =>
  rulesLossless<string>('assertLichessId', value)
export const assertWatchTarget = (
  value: unknown,
  channels: readonly string[],
): { channel: string } | { gameId: string } =>
  rulesLossless<{ channel: string } | { gameId: string }>('assertWatchTarget', value, [...channels])
/** A connected account, or empty for "none". */
export const assertOptionalAccount = (value: unknown): string =>
  rulesLossless<string>('assertOptionalAccount', value)
export const assertOnlineOptions = (value: unknown): OnlineOptions =>
  rulesLossless<OnlineOptions>('assertOnlineOptions', value)
export const assertTheme = (value: unknown): AppTheme =>
  rulesLossless<AppTheme>('assertTheme', value)
export const assertNotification = (value: unknown): NotificationRequest =>
  rulesLossless<NotificationRequest>('assertNotification', value)
export const assertCoreSettings = (value: unknown): CoreSettings =>
  rulesLossless<CoreSettings>('assertCoreSettings', value)
export const assertSettings = (value: unknown): Settings =>
  rulesLossless<Settings>('assertSettings', value)
export const assertPuzzleRequest = (value: unknown): PuzzleRequest =>
  rulesLossless<PuzzleRequest>('assertPuzzleRequest', value)
export const assertPuzzleSolve = (value: unknown): PuzzleSolveRequest =>
  rulesLossless<PuzzleSolveRequest>('assertPuzzleSolve', value)
export const assertDays = (value: unknown): number => rulesLossless<number>('assertDays', value)
export const assertActivityMax = (value: unknown): number =>
  rulesLossless<number>('assertActivityMax', value)
export const assertLocalQuery = (value: unknown): LocalPuzzleQuery =>
  rulesLossless<LocalPuzzleQuery>('assertLocalQuery', value)
export const assertLadderQuery = (value: unknown): LocalLadderQuery =>
  rulesLossless<LocalLadderQuery>('assertLadderQuery', value)
export const assertRunKind = (value: unknown): RunKind =>
  rulesLossless<RunKind>('assertRunKind', value)
export const assertRunInput = (value: unknown): RunInput =>
  rulesLossless<RunInput>('assertRunInput', value)
export const assertBestMoveOptions = (value: unknown): BestMoveOptions =>
  rulesLossless<BestMoveOptions>('assertBestMoveOptions', value)
export const assertAnalysisRequest = (value: unknown): AnalysisRequest =>
  rulesLossless<AnalysisRequest>('assertAnalysisRequest', value)
export const assertReviewRequest = (value: unknown): ReviewRequest =>
  rulesLossless<ReviewRequest>('assertReviewRequest', value)
export const assertReviewKey = (value: unknown): string =>
  rulesLossless<string>('assertReviewKey', value)
export const assertVoiceAttempt = (value: unknown): VoiceAttemptInput =>
  rulesLossless<VoiceAttemptInput>('assertVoiceAttempt', value)
export const assertVoiceUpdate = (value: unknown): VoiceAttemptUpdate =>
  rulesLossless<VoiceAttemptUpdate>('assertVoiceUpdate', value)
export const assertVoiceId = (value: unknown): number =>
  rulesLossless<number>('assertVoiceId', value)
export const assertVoiceLimit = (value: unknown): number =>
  rulesLossless<number>('assertVoiceLimit', value)

export const assertGamePageQuery = (value: unknown): GamePageQuery =>
  rulesLossless<GamePageQuery>('assertGamePageQuery', value)

export function assertStudySyncRequest(
  value: unknown,
): import('@kchess/contracts/types').StudySyncRequest {
  return rulesLossless<import('@kchess/contracts/types').StudySyncRequest>(
    'assertStudySyncRequest',
    value,
  )
}

/** Bounded text for Lichess broadcast search. */
export function assertBroadcastQuery(value: unknown): string | undefined {
  return rulesLossless<string | null>('assertBroadcastQuery', value) ?? undefined
}

export const assertLookupOptions = (value: unknown): LookupOptions =>
  rulesLossless<LookupOptions>('assertLookupOptions', value)
