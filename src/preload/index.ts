import { contextBridge, ipcRenderer } from 'electron'
import type { DesktopApi, OnlineEvent, PuzzleDbProgress } from '../shared/types'

const api: DesktopApi = {
  loadData: () => ipcRenderer.invoke('data:load'),
  saveSettings: (settings) => ipcRenderer.invoke('settings:save', settings),
  addAccount: (username) => ipcRenderer.invoke('account:add', username),
  removeAccount: (username) => ipcRenderer.invoke('account:remove', username),
  syncGames: (username) => ipcRenderer.invoke('games:sync', username),
  gamePgn: (account, id) => ipcRenderer.invoke('games:pgn', account, id),
  cachedProfile: (username) => ipcRenderer.invoke('lichess:cached-profile', username),
  profile: (username) => ipcRenderer.invoke('lichess:profile', username),
  ratingHistory: (username) => ipcRenderer.invoke('lichess:rating', username),
  connectLichess: () => ipcRenderer.invoke('lichess:connect'),
  engineStatus: () => ipcRenderer.invoke('engine:status'),
  chooseEngine: () => ipcRenderer.invoke('engine:choose'),
  installEngine: () => ipcRenderer.invoke('engine:install'),
  deleteEngine: () => ipcRenderer.invoke('engine:delete'),
  bestMove: (moves, level, options) => ipcRenderer.invoke('engine:bestmove', moves, level, options),
  startOnline: (options) => ipcRenderer.invoke('online:start', options),
  resumeOnline: () => ipcRenderer.invoke('online:resume'),
  cancelOnline: () => ipcRenderer.invoke('online:cancel'),
  playOnline: (id, move) => ipcRenderer.invoke('online:move', id, move),
  onlineAction: (id, action) => ipcRenderer.invoke('online:action', id, action),
  clearAccountData: (username) => ipcRenderer.invoke('account:clear-data', username),
  following: () => ipcRenderer.invoke('friends:following'),
  addFriends: (usernames) => ipcRenderer.invoke('friends:add-many', usernames),
  usage: () => ipcRenderer.invoke('usage:report'),
  resetUsage: () => ipcRenderer.invoke('usage:reset'),
  notify: (request) => ipcRenderer.invoke('notify:show', request),
  loadThemes: () => ipcRenderer.invoke('themes:load'),
  openThemesFolder: () => ipcRenderer.invoke('themes:open-folder'),
  openNotificationSettings: () => ipcRenderer.invoke('notify:open-settings'),
  onNotification: (callback) => {
    const listener = (_: unknown, alert: { title: string; body: string }): void => callback(alert)
    ipcRenderer.on('notify:alert', listener)
    return () => ipcRenderer.removeListener('notify:alert', listener)
  },
  presence: (usernames) => ipcRenderer.invoke('online:presence', usernames),
  onOnlineEvent: (callback) => {
    const listener = (_: unknown, event: OnlineEvent): void => callback(event)
    ipcRenderer.on('online:event', listener)
    return () => ipcRenderer.removeListener('online:event', listener)
  },
  puzzleNext: (request) => ipcRenderer.invoke('puzzle:next', request),
  puzzleSolve: (request) => ipcRenderer.invoke('puzzle:solve', request),
  puzzleDaily: () => ipcRenderer.invoke('puzzle:daily'),
  puzzleDashboard: (account, days) => ipcRenderer.invoke('puzzle:dashboard', account, days),
  puzzleActivity: (account, max) => ipcRenderer.invoke('puzzle:activity', account, max),
  stormDashboard: (username, days) => ipcRenderer.invoke('puzzle:storm', username, days),
  puzzleDbStatus: () => ipcRenderer.invoke('puzzledb:status'),
  puzzleDbInstall: () => ipcRenderer.invoke('puzzledb:install'),
  puzzleDbCancel: () => ipcRenderer.invoke('puzzledb:cancel'),
  puzzleDbDelete: () => ipcRenderer.invoke('puzzledb:delete'),
  localPuzzles: (query) => ipcRenderer.invoke('puzzledb:query', query),
  localLadder: (query) => ipcRenderer.invoke('puzzledb:ladder', query),
  onPuzzleDbProgress: (callback) => {
    const listener = (_: unknown, progress: PuzzleDbProgress): void => callback(progress)
    ipcRenderer.on('puzzledb:progress', listener)
    return () => ipcRenderer.removeListener('puzzledb:progress', listener)
  },
  saveRun: (run) => ipcRenderer.invoke('runs:save', run),
  runSummary: (kind) => ipcRenderer.invoke('runs:summary', kind),
  clearRuns: (kind) => ipcRenderer.invoke('runs:clear', kind),
  onOnlineError: (callback) => {
    const listener = (_: unknown, message: string): void => callback(message)
    ipcRenderer.on('online:error', listener)
    return () => ipcRenderer.removeListener('online:error', listener)
  },
}

contextBridge.exposeInMainWorld('kchess', api)
