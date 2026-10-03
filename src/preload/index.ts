import { contextBridge, ipcRenderer } from 'electron'
import type {
  AnalysisUpdate,
  AppUpdateStatus,
  DesktopApi,
  OnlineEvent,
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
  removeAccount: (username) => invoke('removeAccount', username),
  syncGames: (username) => invoke('syncGames', username),
  gamePgn: (account, id) => invoke('gamePgn', account, id),
  cachedProfile: (username) => invoke('cachedProfile', username),
  profile: (username) => invoke('profile', username),
  ratingHistory: (username) => invoke('ratingHistory', username),
  connectLichess: () => invoke('connectLichess'),
  engineStatus: () => invoke('engineStatus'),
  chooseEngine: () => invoke('chooseEngine'),
  installEngine: () => invoke('installEngine'),
  deleteEngine: () => invoke('deleteEngine'),
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
  reviewSummaries: () => invoke('reviewSummaries'),
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
