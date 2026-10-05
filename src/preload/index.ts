import { contextBridge, ipcRenderer } from 'electron'
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
} from '../shared/types'

import {
  IPC_CHANNELS,
  IPC_EVENTS,
  type InvokeMethod,
  type IpcArguments,
  type IpcResult,
} from '../shared/ipc'

function invoke<K extends InvokeMethod>(method: K, ...args: IpcArguments<K>): IpcResult<K> {
  return ipcRenderer.invoke(IPC_CHANNELS[method], ...args) as IpcResult<K>
}

const api: DesktopApi = {
  recordPerformance: (name, milliseconds) => invoke('recordPerformance', name, milliseconds),
  positionLookup: (kind, fen, options) => invoke('positionLookup', kind, fen, options),
  mastersGame: (id) => invoke('mastersGame', id),
  exportGame: (id) => invoke('exportGame', id),
  saveExport: (request) => invoke('saveExport', request),
  lichessStudies: (account) => invoke('lichessStudies', account),
  lichessStudyChapters: (account, id) => invoke('lichessStudyChapters', account, id),
  exportToLichessStudy: (account, studyId, name, pgn) =>
    invoke('exportToLichessStudy', account, studyId, name, pgn),
  cloudEval: (fen, lines) => invoke('cloudEval', fen, lines),
  appUpdateStatus: () => invoke('appUpdateStatus'),
  checkAppUpdate: () => invoke('checkAppUpdate'),
  downloadAppUpdate: () => invoke('downloadAppUpdate'),
  installAppUpdate: () => invoke('installAppUpdate'),
  openAppReleases: () => invoke('openAppReleases'),
  onAppUpdate: (callback) => {
    const listener = (_: unknown, status: AppUpdateStatus): void => callback(status)
    ipcRenderer.on(IPC_EVENTS.appUpdate, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.appUpdate, listener)
  },
  exportDiagnostics: () => invoke('exportDiagnostics'),
  windowMinimize: () => invoke('windowMinimize'),
  windowToggleMaximize: () => invoke('windowToggleMaximize'),
  windowClose: () => invoke('windowClose'),
  windowIsMaximized: () => invoke('windowIsMaximized'),
  onWindowMaximized: (callback) => {
    const listener = (_: unknown, state: { maximized: boolean }): void => callback(state)
    ipcRenderer.on(IPC_EVENTS.windowMaximized, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.windowMaximized, listener)
  },
  loadData: () => invoke('loadData'),
  saveSettings: (settings) => invoke('saveSettings', settings),
  addAccount: (username) => invoke('addAccount', username),
  logout: (username) => invoke('logout', username),
  logoutAll: () => invoke('logoutAll'),
  removeAccount: (username) => invoke('removeAccount', username),
  syncGames: (username) => invoke('syncGames', username),
  gamePgn: (account, id) => invoke('gamePgn', account, id),
  gamePage: (query) => invoke('gamePage', query),
  gameLibraryOverview: () => invoke('gameLibraryOverview'),
  insights: (query) => invoke('insights', query),
  gameRatingHistory: (account) => invoke('gameRatingHistory', account),
  cachedProfile: (username) => invoke('cachedProfile', username),
  profile: (username) => invoke('profile', username),
  ratingHistory: (username) => invoke('ratingHistory', username),
  connectLichess: (look) => invoke('connectLichess', look),
  engineStatus: () => invoke('engineStatus'),
  chooseEngine: () => invoke('chooseEngine'),
  installEngine: () => invoke('installEngine'),
  deleteEngine: () => invoke('deleteEngine'),
  stopEngine: () => invoke('stopEngine'),
  bestMove: (moves, level, options) => invoke('bestMove', moves, level, options),
  startAnalysis: (request) => invoke('startAnalysis', request),
  stopAnalysis: () => invoke('stopAnalysis'),
  onAnalysis: (callback) => {
    const listener = (_: unknown, update: AnalysisUpdate): void => callback(update)
    ipcRenderer.on(IPC_EVENTS.analysis, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.analysis, listener)
  },
  reviewGet: (fen, moves) => invoke('reviewGet', fen, moves),
  reviewRequest: (request) => invoke('reviewRequest', request),
  reviewCancel: (key) => invoke('reviewCancel', key),
  reviewStatus: () => invoke('reviewStatus'),
  reviewSummaries: (ids) => invoke('reviewSummaries', ids),
  onReviewUpdate: (callback) => {
    const listener = (_: unknown, update: ReviewUpdate): void => callback(update)
    ipcRenderer.on(IPC_EVENTS.reviewUpdate, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.reviewUpdate, listener)
  },
  onReviewStatus: (callback) => {
    const listener = (_: unknown, status: ReviewStatus): void => callback(status)
    ipcRenderer.on(IPC_EVENTS.reviewStatus, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.reviewStatus, listener)
  },
  startOnline: (options) => invoke('startOnline', options),
  resumeOnline: () => invoke('resumeOnline'),
  cancelOnline: () => invoke('cancelOnline'),
  playOnline: (id, move) => invoke('playOnline', id, move),
  onlineAction: (id, action) => invoke('onlineAction', id, action),
  onlineChat: (id) => invoke('onlineChat', id),
  sendChat: (id, room, text) => invoke('sendChat', id, room, text),
  stayConnected: (account) => invoke('stayConnected', account),
  challenges: () => invoke('challenges'),
  onChallenges: (callback) => {
    const listener = (_: unknown, list: ChallengeInfo[]): void => callback(list)
    ipcRenderer.on(IPC_EVENTS.challenges, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.challenges, listener)
  },
  acceptChallenge: (id) => invoke('acceptChallenge', id),
  declineChallenge: (id, reason) => invoke('declineChallenge', id, reason),
  cancelChallenge: (id) => invoke('cancelChallenge', id),
  ongoingGames: () => invoke('ongoingGames'),
  openGame: (account, id) => invoke('openGame', account, id),
  tournaments: (account) => invoke('tournaments', account),
  tournament: (system, id, account) => invoke('tournament', system, id, account),
  joinTournament: (system, id, account, password) =>
    invoke('joinTournament', system, id, account, password),
  leaveTournament: (system, id, account) => invoke('leaveTournament', system, id, account),
  tvChannels: () => invoke('tvChannels'),
  playerPerf: (username, perf) => invoke('playerPerf', username, perf),
  crosstable: (a, b) => invoke('crosstable', a, b),
  watch: (target) => invoke('watch', target),
  watchBroadcast: (roundId) => invoke('watchBroadcast', roundId),
  stopWatching: () => invoke('stopWatching'),
  broadcasts: () => invoke('broadcasts'),
  broadcastTour: (id) => invoke('broadcastTour', id),
  onWatchState: (callback) => {
    const listener = (_: unknown, state: WatchState): void => callback(state)
    ipcRenderer.on(IPC_EVENTS.watchState, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.watchState, listener)
  },
  onWatch: (callback) => {
    const listener = (_: unknown, frame: WatchFrame): void => callback(frame)
    ipcRenderer.on(IPC_EVENTS.watch, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.watch, listener)
  },
  onBroadcast: (callback) => {
    const listener = (_: unknown, update: BroadcastUpdate): void => callback(update)
    ipcRenderer.on(IPC_EVENTS.broadcast, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.broadcast, listener)
  },
  onLobbyState: (callback) => {
    const listener = (_: unknown, state: LobbyState): void => callback(state)
    ipcRenderer.on(IPC_EVENTS.lobby, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.lobby, listener)
  },
  onOngoingChanged: (callback) => {
    const listener = (): void => callback()
    ipcRenderer.on(IPC_EVENTS.ongoingChanged, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.ongoingChanged, listener)
  },
  clearAccountData: (username) => invoke('clearAccountData', username),
  following: () => invoke('following'),
  addFriends: (usernames) => invoke('addFriends', usernames),
  usage: () => invoke('usage'),
  resetUsage: () => invoke('resetUsage'),
  notify: (request) => invoke('notify', request),
  loadThemes: () => invoke('loadThemes'),
  openThemesFolder: () => invoke('openThemesFolder'),
  openNotificationSettings: () => invoke('openNotificationSettings'),
  microphoneAccess: (request) => invoke('microphoneAccess', request),
  ensureVoiceModel: () => invoke('ensureVoiceModel'),
  voiceModelStatus: () => invoke('voiceModelStatus'),
  onVoiceModelProgress: (callback) => {
    const listener = (_: unknown, progress: VoiceModelProgress): void => callback(progress)
    ipcRenderer.on(IPC_EVENTS.voiceModelProgress, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.voiceModelProgress, listener)
  },
  openMicrophoneSettings: () => invoke('openMicrophoneSettings'),
  onNotification: (callback) => {
    const listener = (_: unknown, alert: { title: string; body: string }): void => callback(alert)
    ipcRenderer.on(IPC_EVENTS.notification, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.notification, listener)
  },
  presence: (usernames) => invoke('presence', usernames),
  onOnlineState: (callback) => {
    const listener = (_: unknown, state: OnlineConnection): void => callback(state)
    ipcRenderer.on(IPC_EVENTS.onlineState, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.onlineState, listener)
  },
  onOnlineEvent: (callback) => {
    const listener = (_: unknown, event: OnlineEvent): void => callback(event)
    ipcRenderer.on(IPC_EVENTS.online, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.online, listener)
  },
  puzzleNext: (request) => invoke('puzzleNext', request),
  puzzleSolve: (request) => invoke('puzzleSolve', request),
  puzzleDaily: () => invoke('puzzleDaily'),
  puzzleDashboard: (account, days) => invoke('puzzleDashboard', account, days),
  puzzleActivity: (account, max) => invoke('puzzleActivity', account, max),
  stormDashboard: (username, days) => invoke('stormDashboard', username, days),
  puzzleDbStatus: () => invoke('puzzleDbStatus'),
  puzzleDbInstall: () => invoke('puzzleDbInstall'),
  puzzleDbCancel: () => invoke('puzzleDbCancel'),
  puzzleDbDelete: () => invoke('puzzleDbDelete'),
  localPuzzles: (query) => invoke('localPuzzles', query),
  localLadder: (query) => invoke('localLadder', query),
  onPuzzleDbProgress: (callback) => {
    const listener = (_: unknown, progress: PuzzleDbProgress): void => callback(progress)
    ipcRenderer.on(IPC_EVENTS.puzzleProgress, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.puzzleProgress, listener)
  },
  saveRun: (run) => invoke('saveRun', run),
  runSummary: (kind) => invoke('runSummary', kind),
  clearRuns: (kind) => invoke('clearRuns', kind),
  saveVoiceAttempt: (attempt) => invoke('saveVoiceAttempt', attempt),
  updateVoiceAttempt: (id, update) => invoke('updateVoiceAttempt', id, update),
  voiceHistory: (limit) => invoke('voiceHistory', limit),
  clearVoiceHistory: () => invoke('clearVoiceHistory'),
  exportVoiceHistory: () => invoke('exportVoiceHistory'),
  onOnlineError: (callback) => {
    const listener = (_: unknown, message: string): void => callback(message)
    ipcRenderer.on(IPC_EVENTS.error, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.error, listener)
  },
}

contextBridge.exposeInMainWorld('kchess', api)
