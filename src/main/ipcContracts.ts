import * as v from 'valibot'
import type { InvokeMethod, IpcArguments } from '../shared/ipc'
import { POSITION_LOOKUP_KINDS } from '../shared/types'
import {
  assertAction,
  assertActivityMax,
  assertAnalysisRequest,
  assertBestMoveOptions,
  assertChatRoom,
  assertChatText,
  assertDays,
  assertDeclineReason,
  assertExport,
  assertFriendList,
  assertGameId,
  assertGameIds,
  assertGamePageQuery,
  assertInsightsQuery,
  assertLadderQuery,
  assertLevel,
  assertLichessId,
  assertLocalQuery,
  assertMoves,
  assertNotification,
  assertOnlineOptions,
  assertOptionalAccount,
  assertPerfType,
  assertPuzzleRequest,
  assertPuzzleSolve,
  assertReviewKey,
  assertReviewRequest,
  assertRunInput,
  assertRunKind,
  assertSettings,
  assertTournamentId,
  assertTournamentPassword,
  assertTournamentSystem,
  assertUci,
  assertUsername,
  assertUsernames,
  assertVoiceAttempt,
  assertVoiceId,
  assertVoiceLimit,
  assertVoiceUpdate,
  assertWatchTarget,
} from '../shared/validate'
import { oauthLook } from './oauthPage'
import { assertLookupOptions } from './positionLookup'
import { TV_CHANNEL_KEYS } from './spectate'

type Check = (value: unknown) => unknown
type Checks<T extends unknown[]> = { [P in keyof T]-?: Check }
type Contracts = { [K in InvokeMethod]: { min: number; checks: Checks<IpcArguments<K>> } }
const optional =
  (check: Check): Check =>
  (value) =>
    value === undefined ? undefined : check(value)
const boolean = (value: unknown) => v.parse(v.boolean(), value)
const lookupKind = (value: unknown) => v.parse(v.picklist(POSITION_LOOKUP_KINDS), value)
const fen = (value: unknown) => assertAnalysisRequest({ fen: value, lines: 1 })
const lines = (value: unknown) =>
  v.parse(v.pipe(v.number(), v.integer(), v.minValue(1), v.maxValue(5)), value)
const watchTarget = (value: unknown) => assertWatchTarget(value, TV_CHANNEL_KEYS)
const studyId = (value: unknown) => (value === '' ? '' : assertLichessId(value))
const chapterName = (value: unknown) =>
  v.parse(v.pipe(v.string(), v.trim(), v.minLength(1), v.maxLength(100)), value)
const pgn = (value: unknown) =>
  v.parse(
    v.pipe(
      v.string(),
      v.minLength(1),
      v.maxLength(500_000),
      v.check((text) => Boolean(text.trim())),
    ),
    value,
  )
const timingName = (value: unknown) =>
  v.parse(v.picklist(['app.ready', 'board.frame', 'voice.activation']), value)
const timingValue = (value: unknown) =>
  v.parse(v.pipe(v.number(), v.finite(), v.minValue(0), v.maxValue(300_000)), value)

/** Every DesktopApi invocation must declare its arity and a validator for every argument. */
export const IPC_CONTRACTS = {
  recordPerformance: { min: 2, checks: [timingName, timingValue] },
  positionLookup: { min: 2, checks: [lookupKind, fen, assertLookupOptions] },
  saveExport: { min: 1, checks: [assertExport] },
  lichessStudies: { min: 1, checks: [assertUsername] },
  lichessStudyChapters: { min: 2, checks: [assertUsername, assertLichessId] },
  exportToLichessStudy: { min: 4, checks: [assertUsername, studyId, chapterName, pgn] },
  exportGame: { min: 1, checks: [assertGameId] },
  mastersGame: { min: 1, checks: [assertGameId] },
  cloudEval: { min: 2, checks: [fen, lines] },
  exportDiagnostics: { min: 0, checks: [] },
  windowMinimize: { min: 0, checks: [] },
  windowToggleMaximize: { min: 0, checks: [] },
  windowClose: { min: 0, checks: [] },
  windowIsMaximized: { min: 0, checks: [] },
  appUpdateStatus: { min: 0, checks: [] },
  checkAppUpdate: { min: 0, checks: [] },
  downloadAppUpdate: { min: 0, checks: [] },
  installAppUpdate: { min: 0, checks: [] },
  openAppReleases: { min: 0, checks: [] },
  loadData: { min: 0, checks: [] },
  saveSettings: { min: 1, checks: [assertSettings] },
  addAccount: { min: 1, checks: [assertUsername] },
  logout: { min: 1, checks: [assertUsername] },
  logoutAll: { min: 0, checks: [] },
  removeAccount: { min: 1, checks: [assertUsername] },
  syncGames: { min: 0, checks: [optional(assertUsername)] },
  gamePage: { min: 1, checks: [assertGamePageQuery] },
  gameLibraryOverview: { min: 0, checks: [] },
  insights: { min: 1, checks: [assertInsightsQuery] },
  gameRatingHistory: { min: 1, checks: [assertUsername] },
  gamePgn: { min: 2, checks: [assertUsername, assertGameId] },
  cachedProfile: { min: 1, checks: [assertUsername] },
  profile: { min: 1, checks: [assertUsername] },
  ratingHistory: { min: 1, checks: [assertUsername] },
  connectLichess: { min: 0, checks: [oauthLook] },
  engineStatus: { min: 0, checks: [] },
  chooseEngine: { min: 0, checks: [] },
  installEngine: { min: 0, checks: [] },
  deleteEngine: { min: 0, checks: [] },
  stopEngine: { min: 0, checks: [] },
  bestMove: { min: 2, checks: [assertMoves, assertLevel, assertBestMoveOptions] },
  startAnalysis: { min: 1, checks: [assertAnalysisRequest] },
  stopAnalysis: { min: 0, checks: [] },
  reviewGet: { min: 2, checks: [fen, assertMoves] },
  reviewRequest: { min: 1, checks: [assertReviewRequest] },
  reviewCancel: { min: 1, checks: [assertReviewKey] },
  reviewStatus: { min: 0, checks: [] },
  reviewSummaries: { min: 1, checks: [assertGameIds] },
  startOnline: { min: 1, checks: [assertOnlineOptions] },
  resumeOnline: { min: 0, checks: [] },
  cancelOnline: { min: 0, checks: [] },
  playOnline: { min: 2, checks: [assertGameId, assertUci] },
  onlineAction: { min: 2, checks: [assertGameId, assertAction] },
  onlineChat: { min: 1, checks: [assertGameId] },
  sendChat: { min: 3, checks: [assertGameId, assertChatRoom, assertChatText] },
  stayConnected: { min: 1, checks: [assertOptionalAccount] },
  challenges: { min: 0, checks: [] },
  acceptChallenge: { min: 1, checks: [assertGameId] },
  declineChallenge: { min: 2, checks: [assertGameId, assertDeclineReason] },
  cancelChallenge: { min: 1, checks: [assertGameId] },
  ongoingGames: { min: 0, checks: [] },
  openGame: { min: 2, checks: [assertUsername, assertGameId] },
  playerPerf: { min: 2, checks: [assertUsername, assertPerfType] },
  crosstable: { min: 2, checks: [assertUsername, assertUsername] },
  tvChannels: { min: 0, checks: [] },
  watch: { min: 1, checks: [watchTarget] },
  watchBroadcast: { min: 1, checks: [assertLichessId] },
  stopWatching: { min: 0, checks: [] },
  broadcasts: { min: 0, checks: [] },
  broadcastTour: { min: 1, checks: [assertLichessId] },
  tournaments: { min: 1, checks: [assertOptionalAccount] },
  tournament: {
    min: 3,
    checks: [assertTournamentSystem, assertTournamentId, assertOptionalAccount],
  },
  joinTournament: {
    min: 3,
    checks: [assertTournamentSystem, assertTournamentId, assertUsername, assertTournamentPassword],
  },
  leaveTournament: { min: 3, checks: [assertTournamentSystem, assertTournamentId, assertUsername] },
  clearAccountData: { min: 1, checks: [assertUsername] },
  following: { min: 0, checks: [] },
  addFriends: { min: 1, checks: [assertFriendList] },
  usage: { min: 0, checks: [] },
  resetUsage: { min: 0, checks: [] },
  presence: { min: 1, checks: [assertUsernames] },
  notify: { min: 1, checks: [assertNotification] },
  loadThemes: { min: 0, checks: [] },
  openThemesFolder: { min: 0, checks: [] },
  openNotificationSettings: { min: 0, checks: [] },
  microphoneAccess: { min: 1, checks: [boolean] },
  ensureVoiceModel: { min: 0, checks: [] },
  voiceModelStatus: { min: 0, checks: [] },
  openMicrophoneSettings: { min: 0, checks: [] },
  puzzleNext: { min: 1, checks: [assertPuzzleRequest] },
  puzzleSolve: { min: 1, checks: [assertPuzzleSolve] },
  puzzleDaily: { min: 0, checks: [] },
  puzzleDashboard: { min: 2, checks: [assertUsername, assertDays] },
  puzzleActivity: { min: 2, checks: [assertUsername, assertActivityMax] },
  stormDashboard: { min: 2, checks: [assertUsername, assertDays] },
  puzzleDbStatus: { min: 0, checks: [] },
  puzzleDbInstall: { min: 0, checks: [] },
  puzzleDbCancel: { min: 0, checks: [] },
  puzzleDbDelete: { min: 0, checks: [] },
  localPuzzles: { min: 1, checks: [assertLocalQuery] },
  localLadder: { min: 1, checks: [assertLadderQuery] },
  saveRun: { min: 1, checks: [assertRunInput] },
  runSummary: { min: 1, checks: [assertRunKind] },
  clearRuns: { min: 0, checks: [optional(assertRunKind)] },
  saveVoiceAttempt: { min: 1, checks: [assertVoiceAttempt] },
  updateVoiceAttempt: { min: 2, checks: [assertVoiceId, assertVoiceUpdate] },
  voiceHistory: { min: 1, checks: [assertVoiceLimit] },
  clearVoiceHistory: { min: 0, checks: [] },
  exportVoiceHistory: { min: 0, checks: [] },
} satisfies Contracts

export function validateIpcArguments(method: InvokeMethod, args: unknown[]): void {
  const contract = IPC_CONTRACTS[method]
  if (args.length < contract.min || args.length > contract.checks.length)
    throw new Error(`Invalid argument count for ${method}.`)
  for (const [index, check] of contract.checks.entries()) check(args[index])
  if (method === 'reviewGet') assertReviewRequest({ fen: args[0], moves: args[1] })
}
