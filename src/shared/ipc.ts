import type { DesktopApi, OnlineEvent, PuzzleDbProgress } from './types'

export interface IpcEvents {
  'online:event': OnlineEvent
  'online:error': string
  'notify:alert': { title: string; body: string }
  'puzzledb:progress': PuzzleDbProgress
}
export const IPC_EVENTS = {
  online: 'online:event',
  error: 'online:error',
  notification: 'notify:alert',
  puzzleProgress: 'puzzledb:progress',
} as const satisfies Record<string, keyof IpcEvents>

type Subscription = 'onOnlineEvent' | 'onOnlineError' | 'onNotification' | 'onPuzzleDbProgress'
export type InvokeMethod = Exclude<keyof DesktopApi, Subscription>

/** A single channel map shared by main and preload; every request has a DesktopApi signature. */
export const IPC_CHANNELS = {
  loadData: 'data:load',
  saveSettings: 'settings:save',
  addAccount: 'account:add',
  removeAccount: 'account:remove',
  syncGames: 'games:sync',
  gamePgn: 'games:pgn',
  cachedProfile: 'lichess:cached-profile',
  profile: 'lichess:profile',
  ratingHistory: 'lichess:rating',
  connectLichess: 'lichess:connect',
  engineStatus: 'engine:status',
  chooseEngine: 'engine:choose',
  installEngine: 'engine:install',
  deleteEngine: 'engine:delete',
  bestMove: 'engine:bestmove',
  startOnline: 'online:start',
  resumeOnline: 'online:resume',
  cancelOnline: 'online:cancel',
  playOnline: 'online:move',
  onlineAction: 'online:action',
  clearAccountData: 'account:clear-data',
  following: 'friends:following',
  addFriends: 'friends:add-many',
  usage: 'usage:report',
  resetUsage: 'usage:reset',
  notify: 'notify:show',
  loadThemes: 'themes:load',
  openThemesFolder: 'themes:open-folder',
  openNotificationSettings: 'notify:open-settings',
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
} as const satisfies Record<InvokeMethod, string>

export type IpcArguments<K extends InvokeMethod> = Parameters<DesktopApi[K]>
export type IpcResult<K extends InvokeMethod> = ReturnType<DesktopApi[K]>
type UnknownTuple<T extends unknown[]> = { [P in keyof T]: unknown }
export type IpcInput<K extends InvokeMethod> = UnknownTuple<IpcArguments<K>>
