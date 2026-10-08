import type {
  AnalysisUpdate,
  BroadcastUpdate,
  ChallengeInfo,
  CoreApi,
  LobbyState,
  OnlineConnection,
  OnlineEvent,
  PuzzleDbProgress,
  ReviewStatus,
  ReviewUpdate,
  Settings,
  WatchFrame,
  WatchState,
} from './types'

/** Everything the core reports without being asked, keyed like the desktop's IPC events. */
export interface CoreEvents {
  'online:state': OnlineConnection
  'online:event': OnlineEvent
  'online:error': string
  'puzzledb:progress': PuzzleDbProgress
  'engine:analysis': AnalysisUpdate
  'review:update': ReviewUpdate
  'review:status': ReviewStatus
  'challenges:update': ChallengeInfo[]
  'online:lobby': LobbyState
  'online:ongoing-changed': null
  'watch:state': WatchState
  'watch:frame': WatchFrame
  'watch:broadcast': BroadcastUpdate
  /** A challenge that was not pending before; frontends decide how to alert. */
  'challenge:received': ChallengeInfo
  'settings:saved': Settings
}

export type CoreMethod = keyof CoreApi

const methods: Record<CoreMethod, true> = {
  positionLookup: true,
  lichessStudies: true,
  lichessStudyChapters: true,
  syncLichessStudy: true,
  exportToLichessStudy: true,
  exportGame: true,
  mastersGame: true,
  cloudEval: true,
  loadData: true,
  saveSettings: true,
  addAccount: true,
  logout: true,
  logoutAll: true,
  removeAccount: true,
  syncGames: true,
  gamePage: true,
  gameLibraryOverview: true,
  insights: true,
  gameRatingHistory: true,
  gamePgn: true,
  cachedProfile: true,
  profile: true,
  ratingHistory: true,
  connectLichess: true,
  engineStatus: true,
  installEngine: true,
  deleteEngine: true,
  stopEngine: true,
  bestMove: true,
  startAnalysis: true,
  stopAnalysis: true,
  reviewGet: true,
  reviewRequest: true,
  reviewCancel: true,
  reviewStatus: true,
  reviewSummaries: true,
  startOnline: true,
  resumeOnline: true,
  cancelOnline: true,
  playOnline: true,
  onlineAction: true,
  onlineChat: true,
  sendChat: true,
  stayConnected: true,
  challenges: true,
  acceptChallenge: true,
  declineChallenge: true,
  cancelChallenge: true,
  ongoingGames: true,
  openGame: true,
  playerPerf: true,
  crosstable: true,
  recentGames: true,
  sendMessage: true,
  tvChannels: true,
  watch: true,
  watchBroadcast: true,
  stopWatching: true,
  broadcasts: true,
  broadcastTour: true,
  tournaments: true,
  tournament: true,
  joinTournament: true,
  leaveTournament: true,
  createTournament: true,
  clearAccountData: true,
  following: true,
  addFriends: true,
  usage: true,
  resetUsage: true,
  presence: true,
  puzzleNext: true,
  puzzleSolve: true,
  puzzleDaily: true,
  puzzleDashboard: true,
  puzzleActivity: true,
  stormDashboard: true,
  puzzleDbStatus: true,
  puzzleDbInstall: true,
  puzzleDbCancel: true,
  puzzleDbDelete: true,
  localPuzzles: true,
  localLadder: true,
  saveRun: true,
  runSummary: true,
  clearRuns: true,
  saveVoiceAttempt: true,
  updateVoiceAttempt: true,
  voiceHistory: true,
  clearVoiceHistory: true,
  library: true,
  importLibrary: true,
  studyCommand: true,
  saveArchivedGame: true,
  removeArchivedGame: true,
  addMistakes: true,
  answerMistake: true,
  saveSession: true,
  joinedTournaments: true,
  recordRepertoireMiss: true,
  clearRepertoireMisses: true,
}
/** Every CoreApi method; a frontend transport can expose exactly these. */
export const CORE_METHODS = Object.keys(methods) as CoreMethod[]
