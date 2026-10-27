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
  ProfileChange,
  PuzzleDbProgress,
  ReviewStatus,
  ReviewUpdate,
  VoiceModelProgress,
} from '../../contracts/types'
import type { InterfaceZoomState } from '../../contracts/interfaceZoom'

import {
  IPC_CHANNELS,
  IPC_EVENTS,
  type InvokeMethod,
  type IpcArguments,
  type IpcResult,
  invokeErrorText,
} from '../../contracts/ipc'

/** Rejections carry the main process's message alone, ready for the UI. */
function invoke<K extends InvokeMethod>(method: K, ...args: IpcArguments<K>): IpcResult<K> {
  return ipcRenderer.invoke(IPC_CHANNELS[method], ...args).catch((cause: unknown) => {
    throw new Error(invokeErrorText(cause))
  }) as IpcResult<K>
}

const api: DesktopApi = {
  completeQuit: (token, saved) => invoke('completeQuit', token, saved),
  reloadWindow: () => invoke('reloadWindow'),
  onPrepareQuit(callback) {
    const listener = (_event: unknown, token: string): void => callback(token)
    ipcRenderer.on(IPC_EVENTS.prepareQuit, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.prepareQuit, listener)
  },
  recordPerformance: (name, milliseconds) => invoke('recordPerformance', name, milliseconds),
  reportRendererError: (report) => invoke('reportRendererError', report),
  positionLookup: (kind, fen, options) => invoke('positionLookup', kind, fen, options),
  mastersGame: (id) => invoke('mastersGame', id),
  openingDescription: (moves) => invoke('openingDescription', moves),
  openingPopularity: (moves) => invoke('openingPopularity', moves),
  exportGame: (id) => invoke('exportGame', id),
  saveExport: (request) => invoke('saveExport', request),
  lichessStudies: (account) => invoke('lichessStudies', account),
  lichessStudyChapters: (account, id) => invoke('lichessStudyChapters', account, id),
  syncLichessStudy: (request) => invoke('syncLichessStudy', request),
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
  interfaceZoom: () => invoke('interfaceZoom'),
  setInterfaceZoom: (setting) => invoke('setInterfaceZoom', setting),
  onInterfaceZoom: (callback) => {
    const listener = (_: unknown, state: InterfaceZoomState): void => callback(state)
    ipcRenderer.on(IPC_EVENTS.interfaceZoom, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.interfaceZoom, listener)
  },
  loadData: () => invoke('loadData'),
  saveSettings: (settings, base) => invoke('saveSettings', settings, base),
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
  stopAnalysis: (kill) => invoke('stopAnalysis', kill),
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
  tournament: (system, id, account, page) => invoke('tournament', system, id, account, page),
  joinTournament: (system, id, account, password) =>
    invoke('joinTournament', system, id, account, password),
  leaveTournament: (system, id, account) => invoke('leaveTournament', system, id, account),
  createTournament: (account, arena) => invoke('createTournament', account, arena),
  tvChannels: () => invoke('tvChannels'),
  playerPerf: (username, perf) => invoke('playerPerf', username, perf),
  crosstable: (a, b) => invoke('crosstable', a, b),
  recentGames: (username, rated) => invoke('recentGames', username, rated),
  sendMessage: (account, username, text) => invoke('sendMessage', account, username, text),
  relation: (account, username) => invoke('relation', account, username),
  setRelation: (account, username, action) => invoke('setRelation', account, username, action),
  profiles: (usernames) => invoke('profiles', usernames),
  leaderboard: (perf) => invoke('leaderboard', perf),
  searchPlayers: (term) => invoke('searchPlayers', term),
  activity: (username) => invoke('activity', username),
  timeline: (account) => invoke('timeline', account),
  importGame: (account, pgn) => invoke('importGame', account, pgn),
  bookmarkedGames: (account) => invoke('bookmarkedGames', account),
  watch: (target) => invoke('watch', target),
  watchBroadcast: (roundId) => invoke('watchBroadcast', roundId),
  stopWatching: () => invoke('stopWatching'),
  broadcasts: (query) => invoke('broadcasts', query),
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
  onProfileChanged: (callback) => {
    const listener = (_: unknown, change: ProfileChange): void => callback(change)
    ipcRenderer.on(IPC_EVENTS.profileChanged, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.profileChanged, listener)
  },
  onCoreReconnected: (callback) => {
    const listener = (): void => callback()
    ipcRenderer.on(IPC_EVENTS.coreReconnected, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.coreReconnected, listener)
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
  customThemes: () => invoke('customThemes'),
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
  recordPuzzleAttempt: (attempt) => invoke('recordPuzzleAttempt', attempt),
  puzzleProgress: (query) => invoke('puzzleProgress', query),
  saveRun: (run) => invoke('saveRun', run),
  runSummary: (kind) => invoke('runSummary', kind),
  clearRuns: (kind) => invoke('clearRuns', kind),
  saveVoiceAttempt: (attempt) => invoke('saveVoiceAttempt', attempt),
  updateVoiceAttempt: (id, update) => invoke('updateVoiceAttempt', id, update),
  voiceHistory: (limit) => invoke('voiceHistory', limit),
  clearVoiceHistory: () => invoke('clearVoiceHistory'),
  voiceStart: (grammar, sampleRate) => invoke('voiceStart', grammar, sampleRate),
  voiceAccept: (audio) => invoke('voiceAccept', audio),
  voiceFinish: () => invoke('voiceFinish'),
  voiceStop: () => invoke('voiceStop'),
  library: () => invoke('library'),
  importLibrary: (documents) => invoke('importLibrary', documents),
  studyCommand: (command, incremental, basedOn) =>
    invoke('studyCommand', command, incremental, basedOn),
  saveArchivedGame: (game) => invoke('saveArchivedGame', game),
  removeArchivedGame: (id) => invoke('removeArchivedGame', id),
  addMistakes: (reviewKey, color) => invoke('addMistakes', reviewKey, color),
  mistakeGames: () => invoke('mistakeGames'),
  answerMistake: (id, solved) => invoke('answerMistake', id, solved),
  saveSession: (kind, session) => invoke('saveSession', kind, session),
  joinedTournaments: () => invoke('joinedTournaments'),
  recordRepertoireMiss: (key, fen) => invoke('recordRepertoireMiss', key, fen),
  clearRepertoireMisses: (key) => invoke('clearRepertoireMisses', key),
  viewPreferences: () => invoke('viewPreferences'),
  setViewPreference: (key, value) => invoke('setViewPreference', key, value),
  exportVoiceHistory: () => invoke('exportVoiceHistory'),
  onOnlineError: (callback) => {
    const listener = (_: unknown, message: string): void => callback(message)
    ipcRenderer.on(IPC_EVENTS.error, listener)
    return () => ipcRenderer.removeListener(IPC_EVENTS.error, listener)
  },
}

contextBridge.exposeInMainWorld('kchess', api)
