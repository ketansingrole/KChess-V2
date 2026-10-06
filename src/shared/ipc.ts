import type {
  BroadcastUpdate,
  WatchFrame,
  WatchState,
  ChallengeInfo,
  LobbyState,
  AnalysisUpdate,
  AppUpdateStatus,
  DesktopApi,
  OnlineEvent,
  OnlineConnection,
  PuzzleDbProgress,
  ReviewStatus,
  ReviewUpdate,
  VoiceModelProgress,
} from './types'

export interface IpcEvents {
  'online:state': OnlineConnection
  'online:event': OnlineEvent
  'online:error': string
  'notify:alert': { title: string; body: string }
  'puzzledb:progress': PuzzleDbProgress
  'app:update': AppUpdateStatus
  'voice:model-progress': VoiceModelProgress
  'window:maximized-changed': { maximized: boolean }
  'engine:analysis': AnalysisUpdate
  'review:update': ReviewUpdate
  'review:status': ReviewStatus
  'challenges:update': ChallengeInfo[]
  'online:lobby': LobbyState
  'online:ongoing-changed': null
  'watch:state': WatchState
  'watch:frame': WatchFrame
  'watch:broadcast': BroadcastUpdate
}
export const IPC_EVENTS = {
  onlineState: 'online:state',
  online: 'online:event',
  error: 'online:error',
  notification: 'notify:alert',
  puzzleProgress: 'puzzledb:progress',
  appUpdate: 'app:update',
  voiceModelProgress: 'voice:model-progress',
  windowMaximized: 'window:maximized-changed',
  analysis: 'engine:analysis',
  reviewUpdate: 'review:update',
  reviewStatus: 'review:status',
  challenges: 'challenges:update',
  lobby: 'online:lobby',
  ongoingChanged: 'online:ongoing-changed',
  watchState: 'watch:state',
  watch: 'watch:frame',
  broadcast: 'watch:broadcast',
} as const satisfies Record<string, keyof IpcEvents>

type Subscription =
  | 'onAppUpdate'
  | 'onOnlineEvent'
  | 'onOnlineState'
  | 'onOnlineError'
  | 'onNotification'
  | 'onPuzzleDbProgress'
  | 'onVoiceModelProgress'
  | 'onWindowMaximized'
  | 'onAnalysis'
  | 'onReviewUpdate'
  | 'onReviewStatus'
  | 'onChallenges'
  | 'onLobbyState'
  | 'onOngoingChanged'
  | 'onWatchState'
  | 'onWatch'
  | 'onBroadcast'
export type InvokeMethod = Exclude<keyof DesktopApi, Subscription>

/** A single channel map shared by main and preload; every request has a DesktopApi signature. */
export const IPC_CHANNELS = {
  recordPerformance: 'diagnostics:performance',
  reportRendererError: 'diagnostics:renderer-error',
  positionLookup: 'analysis:position-lookup',
  mastersGame: 'analysis:masters-game',
  exportGame: 'games:export',
  saveExport: 'games:save-export',
  lichessStudies: 'studies:list',
  lichessStudyChapters: 'studies:chapters',
  exportToLichessStudy: 'studies:export',
  cloudEval: 'analysis:cloud-eval',
  appUpdateStatus: 'app-update:status',
  checkAppUpdate: 'app-update:check',
  downloadAppUpdate: 'app-update:download',
  installAppUpdate: 'app-update:install',
  openAppReleases: 'app-update:releases',
  loadData: 'data:load',
  saveSettings: 'settings:save',
  addAccount: 'account:add',
  logout: 'account:logout',
  logoutAll: 'account:logout-all',
  removeAccount: 'account:remove',
  syncGames: 'games:sync',
  gamePgn: 'games:pgn',
  gamePage: 'games:page',
  gameLibraryOverview: 'games:overview',
  insights: 'games:insights',
  gameRatingHistory: 'games:ratings',
  cachedProfile: 'lichess:cached-profile',
  profile: 'lichess:profile',
  ratingHistory: 'lichess:rating',
  connectLichess: 'lichess:connect',
  engineStatus: 'engine:status',
  chooseEngine: 'engine:choose',
  installEngine: 'engine:install',
  deleteEngine: 'engine:delete',
  bestMove: 'engine:bestmove',
  stopEngine: 'engine:stop',
  startAnalysis: 'engine:analysis-start',
  stopAnalysis: 'engine:analysis-stop',
  reviewGet: 'review:get',
  reviewRequest: 'review:request',
  reviewCancel: 'review:cancel',
  reviewStatus: 'review:status-get',
  reviewSummaries: 'review:summaries',
  startOnline: 'online:start',
  resumeOnline: 'online:resume',
  cancelOnline: 'online:cancel',
  playOnline: 'online:move',
  onlineAction: 'online:action',
  onlineChat: 'online:chat',
  sendChat: 'online:chat-send',
  stayConnected: 'online:stay-connected',
  challenges: 'challenges:list',
  acceptChallenge: 'challenges:accept',
  declineChallenge: 'challenges:decline',
  cancelChallenge: 'challenges:cancel',
  ongoingGames: 'online:ongoing',
  openGame: 'online:open',
  tournaments: 'tournaments:list',
  tournament: 'tournaments:get',
  joinTournament: 'tournaments:join',
  leaveTournament: 'tournaments:leave',
  tvChannels: 'watch:tv-channels',
  playerPerf: 'players:perf',
  crosstable: 'players:crosstable',
  watch: 'watch:start',
  watchBroadcast: 'watch:broadcast-start',
  stopWatching: 'watch:stop',
  broadcasts: 'watch:broadcasts',
  broadcastTour: 'watch:broadcast-tour',
  clearAccountData: 'account:clear-data',
  following: 'friends:following',
  addFriends: 'friends:add-many',
  usage: 'usage:report',
  resetUsage: 'usage:reset',
  notify: 'notify:show',
  loadThemes: 'themes:load',
  openThemesFolder: 'themes:open-folder',
  openNotificationSettings: 'notify:open-settings',
  microphoneAccess: 'voice:microphone-access',
  ensureVoiceModel: 'voice:model-ensure',
  voiceModelStatus: 'voice:model-status',
  openMicrophoneSettings: 'voice:open-settings',
  presence: 'online:presence',
  puzzleNext: 'puzzle:next',
  puzzleSolve: 'puzzle:solve',
  puzzleDaily: 'puzzle:daily',
  puzzleDashboard: 'puzzle:dashboard',
  puzzleActivity: 'puzzle:activity',
  stormDashboard: 'puzzle:storm',
  puzzleDbStatus: 'puzzledb:status',
  puzzleDbInstall: 'puzzledb:install',
  puzzleDbCancel: 'puzzledb:cancel',
  puzzleDbDelete: 'puzzledb:delete',
  localPuzzles: 'puzzledb:query',
  localLadder: 'puzzledb:ladder',
  saveRun: 'runs:save',
  runSummary: 'runs:summary',
  clearRuns: 'runs:clear',
  exportDiagnostics: 'diagnostics:export',
  saveVoiceAttempt: 'voice:log-save',
  updateVoiceAttempt: 'voice:log-update',
  voiceHistory: 'voice:log',
  clearVoiceHistory: 'voice:log-clear',
  exportVoiceHistory: 'voice:log-export',
  windowMinimize: 'window:minimize',
  windowToggleMaximize: 'window:toggle-maximize',
  windowClose: 'window:close',
  windowIsMaximized: 'window:is-maximized',
} as const satisfies Record<InvokeMethod, string>

export type IpcArguments<K extends InvokeMethod> = Parameters<DesktopApi[K]>
export type IpcResult<K extends InvokeMethod> = ReturnType<DesktopApi[K]>
type UnknownTuple<T extends unknown[]> = { [P in keyof T]: unknown }
export type IpcInput<K extends InvokeMethod> = UnknownTuple<IpcArguments<K>>
