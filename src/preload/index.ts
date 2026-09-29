import { contextBridge, ipcRenderer } from 'electron'
import type { DesktopApi, OnlineEvent } from '../shared/types'

const api: DesktopApi = {
  loadData: () => ipcRenderer.invoke('data:load'),
  saveSettings: settings => ipcRenderer.invoke('settings:save', settings),
  addAccount: username => ipcRenderer.invoke('account:add', username),
  removeAccount: username => ipcRenderer.invoke('account:remove', username),
  syncGames: username => ipcRenderer.invoke('games:sync', username),
  profile: username => ipcRenderer.invoke('lichess:profile', username),
  ratingHistory: username => ipcRenderer.invoke('lichess:rating', username),
  connectLichess: () => ipcRenderer.invoke('lichess:connect'),
  engineStatus: path => ipcRenderer.invoke('engine:status', path),
  chooseEngine: () => ipcRenderer.invoke('engine:choose'),
  installEngine: () => ipcRenderer.invoke('engine:install'),
  bestMove: (moves, level, path) => ipcRenderer.invoke('engine:bestmove', moves, level, path),
  startOnline: options => ipcRenderer.invoke('online:start', options),
  resumeOnline: () => ipcRenderer.invoke('online:resume'),
  cancelOnline: () => ipcRenderer.invoke('online:cancel'),
  playOnline: (id, move) => ipcRenderer.invoke('online:move', id, move),
  onlineAction: (id, action) => ipcRenderer.invoke('online:action', id, action),
  onOnlineEvent: callback => {
    const listener = (_: unknown, event: OnlineEvent): void => callback(event)
    ipcRenderer.on('online:event', listener)
    return () => ipcRenderer.removeListener('online:event', listener)
  },
  onOnlineError: callback => {
    const listener = (_: unknown, message: string): void => callback(message)
    ipcRenderer.on('online:error', listener)
    return () => ipcRenderer.removeListener('online:error', listener)
  }
}

contextBridge.exposeInMainWorld('kchess', api)
