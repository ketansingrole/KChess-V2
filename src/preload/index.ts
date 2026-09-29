import { contextBridge, ipcRenderer } from 'electron'
import type { DesktopApi, OnlineEvent } from '../shared/types'

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
  bestMove: (moves, level) => ipcRenderer.invoke('engine:bestmove', moves, level),
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
  presence: (usernames) => ipcRenderer.invoke('online:presence', usernames),
  onOnlineEvent: (callback) => {
    const listener = (_: unknown, event: OnlineEvent): void => callback(event)
    ipcRenderer.on('online:event', listener)
    return () => ipcRenderer.removeListener('online:event', listener)
  },
  onOnlineError: (callback) => {
    const listener = (_: unknown, message: string): void => callback(message)
    ipcRenderer.on('online:error', listener)
    return () => ipcRenderer.removeListener('online:error', listener)
  },
}

contextBridge.exposeInMainWorld('kchess', api)
